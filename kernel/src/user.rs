//! BSP processes: private ELF mappings, spawn/wait, blocking I/O and round-robin.

use alloc::{sync::Arc, vec::Vec};
mod handles;
mod i386;
pub mod jobs;
mod linux;
use core::{
    arch::global_asm,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use handles::{Handle, Pipe};
use x86_64::{
    registers::{
        control::{Cr0, Cr0Flags, Cr3, Cr4, Cr4Flags},
        model_specific::{Efer, EferFlags},
    },
    structures::{
        idt::InterruptStackFrame,
        paging::{
            FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags as Flags,
            PhysFrame, Size4KiB,
        },
    },
    PrivilegeLevel, VirtAddr,
};

use crate::{
    arch::x86_64::gdt,
    compat::elf::{Executable, Segment},
    drivers::console,
    mm::phys,
};

const PAGE: u64 = 4096;
const USER_BASE: u64 = 0x80_0000_0000; // PML4 slot 1, separate from the kernel identity map
const IMAGE_END: u64 = USER_BASE + 128 * 1024 * 1024;
const STACK_TOP: u64 = USER_BASE + 256 * 1024 * 1024;
const STACK_BOTTOM: u64 = STACK_TOP - 1024 * 1024;

static ACTIVE: AtomicPtr<Program> = AtomicPtr::new(ptr::null_mut());

global_asm!(
    include_str!("user_entry.asm"),
    user_cs = const gdt::USER_CS64.0,
    user_ds = const gdt::USER_DS.0,
    trap_bytes = const if cfg!(feature = "live") { 64 * 1024 } else { 256 * 1024 },
    options(att_syntax),
);

unsafe extern "C" {
    fn enter_user(context:*const Context) -> i64;
    fn leave_user(status: i64) -> !;
    pub fn user_syscall();
    pub fn user_timer();
    fn resume_user(context: *const Context) -> !;
}

// Layout matches the 15 pushes in user_timer, followed by the CPU iret frame.
#[repr(C)]
#[derive(Clone, Copy)]
struct Context {
    registers: [u64; 15],
    rip: u64,
    cs: u64,
    flags: u64,
    rsp: u64,
    ss: u64,
}
impl Context {
    fn new(entry: u64) -> Self {
        Self {
            registers: [0; 15],
            rip: entry,
            cs: gdt::USER_CS64.0 as u64,
            flags: 0x202,
            rsp: STACK_TOP,
            ss: gdt::USER_DS.0 as u64,
        }
    }
}
#[repr(C, align(16))]
struct Fpu([u8; 512]);
impl Fpu {
    fn new() -> Self {
        let mut state = Self([0; 512]);
        state.0[0..2].copy_from_slice(&0x037fu16.to_le_bytes());
        state.0[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        state
    }
    unsafe fn save(&mut self) {
        core::arch::asm!("fxsave64 [{}]", in(reg) self.0.as_mut_ptr(), options(nostack));
    }
    unsafe fn restore(&self) {
        core::arch::asm!("fxrstor64 [{}]", in(reg) self.0.as_ptr(), options(nostack));
    }
}
struct Batch {
    programs: Vec<Program>,
    current: usize,
    next_pid: u64,
    native_init: bool,
    kernel_root: PhysFrame,
    input: [u8; 256],
    input_head: usize,
    input_len: usize,
    slice_end: Option<u64>,
}
static BATCH: AtomicPtr<Batch> = AtomicPtr::new(ptr::null_mut());

struct Frames(Vec<PhysFrame>);

unsafe impl FrameAllocator<Size4KiB> for Frames {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.0.try_reserve(1).ok()?;
        let frame = phys::allocate_user_frame()?;
        self.0.push(frame);
        Some(frame)
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        for frame in self.0.drain(..) {
            phys::deallocate_frame(frame);
        }
    }
}

pub(crate) struct Program {
    frames: Frames,
    root: PhysFrame,
    pages: Vec<(u64, PhysFrame, Flags)>,
    descriptors: [Option<Arc<Handle>>; 64],
    pid: u64,
    parent: u64,
    reaped: bool,
    blocked: Option<(u64, u64, u64, u64)>,
    context: Context,
    fpu: Fpu,
    status: Option<i64>,
    linux: linux::State,
    i386:Option<i386::State>,
}

impl Program {
    fn new() -> Result<Self, &'static str> {
        let mut frames = Frames(Vec::new());
        let root = frames.allocate_frame().ok_or("out of memory")?;
        let batch = BATCH.load(Ordering::Acquire);
        let kernel_root = if batch.is_null() {
            Cr3::read().0
        } else {
            unsafe { (*batch).kernel_root }
        };
        let old = kernel_root.start_address().as_u64() as *const PageTable;
        let new = root.start_address().as_u64() as *mut PageTable;
        unsafe {
            if !(&*old)[1].is_unused() {
                return Err("user address space is unavailable");
            }
            // Keep the existing supervisor-only mappings, including the low kernel mapping.
            ptr::copy_nonoverlapping(old, new, 1);
        }
        Ok(Self {
            frames,
            root,
            pages: Vec::new(),
            descriptors: core::array::from_fn(|i| match i {
                0 => Some(Arc::new(Handle::ConsoleRead)),
                1 | 2 => Some(Arc::new(Handle::ConsoleWrite)),
                _ => None,
            }),
            pid: 0,
            parent: 0,
            reaped: false,
            blocked: None,
            context: Context::new(0),
            fpu: Fpu::new(),
            status: None,
            linux: linux::State::new(), i386:None,
        })
    }

    fn page(&mut self, address: u64, flags: Flags) -> Result<PhysFrame, &'static str> {
        if self.pages.iter().any(|(page, _, _)| *page == address) {
            return Err("overlapping ELF pages");
        }
        self.pages.try_reserve(1).map_err(|_| "out of memory")?;
        let frame = self.frames.allocate_frame().ok_or("out of memory")?;
        unsafe {
            ptr::write_bytes(frame.start_address().as_u64() as *mut u8, 0, PAGE as usize);
            let root = &mut *(self.root.start_address().as_u64() as *mut PageTable);
            let mut mapper = OffsetPageTable::new(root, VirtAddr::zero());
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
            mapper
                .map_to(
                    page,
                    frame,
                    flags | Flags::PRESENT | Flags::USER_ACCESSIBLE,
                    &mut self.frames,
                )
                .map_err(|error| match error {
                    x86_64::structures::paging::mapper::MapToError::FrameAllocationFailed => {
                        "out of memory"
                    }
                    _ => "could not map a user page",
                })?
                .ignore();
        }
        self.pages.push((address, frame, flags));
        Ok(frame)
    }

    fn load(&mut self, segment: &Segment<'_>) -> Result<(), &'static str> {
        let end = segment
            .address
            .checked_add(segment.size)
            .ok_or("ELF segment overflow")?;
        if segment.address < USER_BASE || end > IMAGE_END {
            return Err("ELF segment outside the bOS application region");
        }
        self.load_range(segment)
    }
    fn load_range(&mut self,segment:&Segment<'_>)->Result<(), &'static str>{
        let end=segment.address.checked_add(segment.size).ok_or("ELF segment overflow")?;
        if segment.size == 0 {
            return Ok(());
        }
        let mut flags = Flags::empty();
        flags.set(Flags::WRITABLE, segment.writable);
        flags.set(Flags::NO_EXECUTE, !segment.executable);
        let file_end = segment.address + segment.bytes.len() as u64;
        for address in ((segment.address & !(PAGE - 1))..end).step_by(PAGE as usize) {
            let frame = self.page(address, flags)?;
            let from = segment.address.max(address);
            let to = file_end.min(address + PAGE);
            if from < to {
                let offset = (from - segment.address) as usize;
                let destination = frame.start_address().as_u64() + from - address;
                unsafe {
                    ptr::copy_nonoverlapping(
                        segment.bytes.as_ptr().add(offset),
                        destination as *mut u8,
                        (to - from) as usize,
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_write(&self, address: u64, length: usize) -> Result<(), i64> {
        let end = address.checked_add(length as u64).ok_or(-14i64)?;
        let mut cursor = address;
        while cursor < end {
            let base = cursor & !(PAGE - 1);
            if !self
                .pages
                .iter()
                .any(|(p, _, flags)| *p == base && flags.contains(Flags::WRITABLE))
            {
                return Err(-14);
            }
            cursor = (base + PAGE).min(end);
        }
        Ok(())
    }

    fn write_user(&self, address: u64, input: &[u8]) -> Result<(), i64> {
        self.validate_write(address, input.len())?;
        let mut offset = 0;
        while offset < input.len() {
            let cursor = address + offset as u64;
            let base = cursor & !(PAGE - 1);
            let (_, frame, _) = self
                .pages
                .iter()
                .find(|(p, _, _)| *p == base)
                .ok_or(-14i64)?;
            let local = cursor - base;
            let length = ((PAGE - local) as usize).min(input.len() - offset);
            unsafe {
                ptr::copy_nonoverlapping(
                    input.as_ptr().add(offset),
                    (frame.start_address().as_u64() + local) as *mut u8,
                    length,
                );
            }
            offset += length;
        }
        Ok(())
    }

    fn path(&self, address: u64) -> Result<alloc::string::String, i64> {
        let mut bytes = Vec::new();
        for offset in 0..4096 {
            let mut byte = [0];
            self.read_user(address.checked_add(offset).ok_or(-14i64)?, &mut byte)
                .map_err(|_| -14i64)?;
            if byte[0] == 0 {
                return alloc::string::String::from_utf8(bytes).map_err(|_| -22i64);
            }
            bytes.try_reserve(1).map_err(|_| -12i64)?;
            bytes.push(byte[0]);
        }
        Err(-36)
    }

    fn descriptor(&self, fd: u64) -> Result<Arc<Handle>, i64> {
        self.descriptors
            .get(fd as usize)
            .and_then(Clone::clone)
            .ok_or(-9)
    }

    pub(crate) fn file_call(&mut self, number: u64, a: u64, b: u64, c: u64) -> Result<i64, i64> {
        use crate::fs::{self, VFS};
        let error = |e| crate::syscall::fs_err(e) as i64;
        match number {
            74 => {
                let fd = self.descriptor(a)?;
                if !matches!(&*fd, Handle::File(_)) {
                    return Err(-22);
                }
                VFS.lock().sync().map_err(error)?;
                Ok(0)
            }
            162 => {
                VFS.lock().sync().map_err(error)?;
                Ok(0)
            }
            22 => {
                self.validate_write(a, 8)?;
                let slots: Vec<_> = (3..64)
                    .filter(|i| self.descriptors[*i].is_none())
                    .take(2)
                    .collect();
                if slots.len() != 2 {
                    return Err(-24);
                }
                let (read, write) = Pipe::pair();
                let mut pair = [0u8; 8];
                pair[..4].copy_from_slice(&(slots[0] as u32).to_le_bytes());
                pair[4..].copy_from_slice(&(slots[1] as u32).to_le_bytes());
                self.write_user(a, &pair)?;
                self.descriptors[slots[0]] = Some(read);
                self.descriptors[slots[1]] = Some(write);
                Ok(0)
            }
            33 => {
                if b >= 64 {
                    return Err(-9);
                }
                let handle = self.descriptor(a)?;
                self.descriptors[b as usize] = Some(handle);
                Ok(b as i64)
            }
            39 => Ok(self.pid as i64),
            110 => Ok(self.parent as i64),
            2 => {
                let path = self.path(a)?;
                let allowed = fs::O_ACCMODE
                    | fs::O_CREAT
                    | fs::O_EXCL
                    | fs::O_TRUNC
                    | fs::O_APPEND
                    | fs::O_DIRECTORY;
                if b > u32::MAX as u64
                    || b as u32 & !allowed != 0
                    || b as u32 & fs::O_ACCMODE == fs::O_ACCMODE
                {
                    return Err(-22);
                }
                let slot = (3..self.descriptors.len())
                    .find(|i| self.descriptors[*i].is_none())
                    .ok_or(-24i64)?;
                self.descriptors[slot] = Some(Arc::new(Handle::File(
                    VFS.lock().open(&path, b as u32).map_err(error)?,
                )));
                Ok(slot as i64)
            }
            3 => {
                let fd = self.descriptor(a)?;
                drop(fd);
                self.descriptors[a as usize] = None;
                Ok(0)
            }
            8 => {
                let fd = self.descriptor(a)?;
                if c > 2 {
                    return Err(-22);
                }
                fd.seek(b as i64, c as u32)
            }
            0 | 1 => {
                let fd = self.descriptor(a)?;
                if c > 4096 {
                    return Err(-22);
                }
                let mut buffer = [0u8; 4096];
                let buffer = &mut buffer[..c as usize];
                if number == 0 {
                    // Validate the whole destination before advancing the file offset.
                    self.validate_write(b, buffer.len())?;
                    let n = fd.read(buffer)?;
                    self.write_user(b, &buffer[..n])?;
                    Ok(n as i64)
                } else {
                    self.read_user(b, buffer).map_err(|_| -14i64)?;
                    Ok(fd.write(buffer)? as i64)
                }
            }
            _ => Err(-38),
        }
    }

    pub(crate) fn read_user(&self, address: u64, output: &mut [u8]) -> Result<(), ()> {
        address.checked_add(output.len() as u64).ok_or(())?;
        let mut address = address;
        let mut offset = 0;
        while offset < output.len() {
            let page = address & !(PAGE - 1);
            let (_, frame, _) = self
                .pages
                .iter()
                .find(|(base, _, _)| *base == page)
                .ok_or(())?;
            let local = address - page;
            let length = ((PAGE - local) as usize).min(output.len() - offset);
            let source = (frame.start_address().as_u64() + local) as *const u8;
            unsafe {
                ptr::copy_nonoverlapping(source, output.as_mut_ptr().add(offset), length);
            }
            address += length as u64;
            offset += length;
        }
        Ok(())
    }
}

fn load_program(data: &[u8]) -> Result<Program, &'static str> {
    load_args(data,&["program"])
}
fn load_args(data:&[u8],args:&[&str])->Result<Program,&'static str>{
    if data.get(..5)==Some(b"\x7fELF\x01"){return i386::load(data,args);}
    let executable = Executable::parse(data)?;
    let mut program = Program::new()?;
    let mut valid_entry = false;
    for segment in executable.segments() {
        let segment = segment?;
        program.load(&segment)?;
        valid_entry |= segment.executable
            && executable.entry >= segment.address
            && executable.entry < segment.address + segment.bytes.len() as u64;
    }
    if !valid_entry {
        return Err("entry point is outside executable file data");
    }
    for address in (STACK_BOTTOM..STACK_TOP).step_by(PAGE as usize) {
        program.page(address, Flags::WRITABLE | Flags::NO_EXECUTE)?;
    }

    program.context.rip = executable.entry;
    if executable.linux {
        linux::initialize(&mut program, &executable, &[alloc::string::String::from("/bin/nash")], &linux::environment())?;
    }
    Ok(program)
}

pub fn run(data: &[u8]) -> Result<i64, &'static str> {
    Ok(run_many(&[data])?[0])
}

/// Bounded BSP scheduler. All images are validated before any user code runs.
/// Kernel execution stays non-preemptible until blocking syscall continuations land.
pub fn run_init(image: &[u8]) -> Result<Vec<i64>, &'static str> {
    run_session(&[image], true,&[])
}
pub fn run_many(images: &[&[u8]]) -> Result<Vec<i64>, &'static str> {
    run_session(images, false,&[])
}
pub fn run_args(image:&[u8],args:&[&str])->Result<i64,&'static str>{Ok(run_session(&[image],false,args)?[0])}
fn run_session(images: &[&[u8]], native_init: bool,args:&[&str]) -> Result<Vec<i64>, &'static str> {
    if images.is_empty() || images.len() > 16 {
        return Err("expected 1 to 16 programs");
    }
    if !BATCH.load(Ordering::Acquire).is_null() {
        return Err("nested user launch");
    }
    if crate::arch::x86_64::apic::timer_frequency() == 0 {
        return Err("APIC timer unavailable");
    }
    let mut batch = Batch {
        programs: Vec::new(),
        current: 0,
        next_pid: 1,
        native_init,
        kernel_root: Cr3::read().0,
        input: [0; 256],
        input_head: 0,
        input_len: 0,slice_end:None,
    };
    batch
        .programs
        .try_reserve(64)
        .map_err(|_| "out of memory")?;
    for image in images {
        let mut program = if args.is_empty(){load_program(image)?}else{load_args(image,args)?};
        program.pid = batch.next_pid;
        batch.next_pid += 1;
        batch.programs.push(program);
    }
    let mut statuses = Vec::new();
    statuses
        .try_reserve(images.len())
        .map_err(|_| "out of memory")?;
    run_batch(&mut batch);
    for program in batch.programs.iter().take(images.len()) {
        statuses.push(program.status.unwrap_or(-1));
    }
    Ok(statuses)
}

fn run_batch(batch:&mut Batch){
    let (previous_root, flags) = Cr3::read();
    let cr0 = Cr0::read();
    let cr4 = Cr4::read();
    let efer = Efer::read();
    let mut kernel_fpu = Fpu::new();
    BATCH.store(batch as *mut Batch, Ordering::Release);
    ACTIVE.store(ptr::addr_of_mut!(batch.programs[batch.current]), Ordering::Release);
    unsafe {
        linux::enable_syscall();
        Cr0::update(|f| f.remove(Cr0Flags::TASK_SWITCHED | Cr0Flags::EMULATE_COPROCESSOR));
        Cr4::update(|f| {
            f.insert(Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
            // FXSAVE covers x87/SSE only. Do not expose unsaved AVX state.
            f.remove(Cr4Flags::OSXSAVE);
        });
        kernel_fpu.save();
        batch.programs[batch.current].fpu.restore();
        Cr3::write(batch.programs[batch.current].root, flags);
        let old_period=crate::arch::x86_64::apic::active_period();
        crate::arch::x86_64::apic::set_timer_periodic(
            crate::arch::x86_64::apic::timer_frequency() / 100,
        );
        linux::set_fs(batch.programs[batch.current].linux.fs);
        enter_user(&batch.programs[batch.current].context);
        if let Some(period)=old_period{crate::arch::x86_64::apic::set_timer_periodic(period);}else{crate::arch::x86_64::apic::stop_timer();}
        Cr3::write(previous_root, flags);
        kernel_fpu.restore();
        Cr4::write(cr4);
        Cr0::write(cr0);
        Efer::write(efer);
        linux::set_fs(0);
    }
    ACTIVE.store(ptr::null_mut(), Ordering::Release);
    BATCH.store(ptr::null_mut(), Ordering::Release);
}

fn poll_input(batch: &mut Batch) {
    for _ in 0..32 {
        let Some(byte) =
            crate::arch::x86_64::serial::try_read_byte().or_else(crate::bsh::native_keyboard_byte)
        else {
            break;
        };
        if byte == 3 {
            // Keep PID 1 alive in the native boot session; batch roots have parent 0.
            let native_init = batch.native_init;
            for p in &mut batch.programs {
                if p.status.is_none() && (!native_init || p.pid != 1) {
                    finish(p, 130);
                }
            }
            if native_init {
                for p in &mut batch.programs {
                    if p.pid != 1 {
                        p.parent = 1;
                    }
                }
            }
        } else if batch.input_len < 256 {
            let tail = (batch.input_head + batch.input_len) % 256;
            batch.input[tail] = byte;
            batch.input_len += 1;
        }
    }
}
fn input_byte() -> Option<u8> {
    unsafe {
        let batch = BATCH.load(Ordering::Acquire);
        if batch.is_null() || (*batch).input_len == 0 {
            return None;
        }
        let head = (*batch).input_head;
        let byte = (*batch).input[head];
        (*batch).input_head = (head + 1) % 256;
        (*batch).input_len -= 1;
        Some(byte)
    }
}
fn finish(program: &mut Program, status: i64) {
    program.status = Some(status);
    program.blocked = None;
    for fd in &mut program.descriptors {
        *fd = None;
    }
}

unsafe fn select_next(batch: *mut Batch) -> *const Context {
    loop {
        if let Some(end)=(*batch).slice_end {if crate::boottime::elapsed_ms()>=end {leave_user(0);}}else{poll_input(&mut *batch);}
        crate::drivers::net::poll();
        for distance in 1..=(*batch).programs.len() {
            let index = ((*batch).current + distance) % (*batch).programs.len();
            if (*batch).programs[index].status.is_some() {
                continue;
            }
            if let Some((nr, a, b, c)) = (*batch).programs[index].blocked {
                let result = if (*batch).programs[index].linux.enabled {
                    linux::call(batch, index, nr, a, b, c)
                } else { process_call(batch, index, nr, a, b, c) };
                if result == -11 {
                    continue;
                }
                (*batch).programs[index].blocked = None;
                (*batch).programs[index].context.registers[14] = result as u64;
            }
            (*batch).current = index;
            let program = &mut (*batch).programs[index];
            ACTIVE.store(program, Ordering::Release);
            let flags = Cr3::read().1;
            Cr3::write(program.root, flags);
            program.fpu.restore();
            linux::set_fs(program.linux.fs);
            return &program.context;
        }
        if (*batch).slice_end.is_some()||(*batch).programs.iter().all(|p| p.status.is_some()) {
            leave_user(0);
        }
        // IRQs do not switch kernel continuations. Timer wakes idle to check input.
        core::arch::asm!("sti; hlt; cli", options(nomem, nostack));
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn switch_user_timer(context: *const Context) -> *const Context {
    crate::arch::x86_64::apic::send_eoi();
    let batch = BATCH.load(Ordering::Acquire);
    if batch.is_null() || (*context).cs & 3 != 3 {
        return context;
    }
    Cr3::write((*batch).kernel_root,Cr3::read().1);
    let current = &mut (*batch).programs[(*batch).current];
    current.context = *context;
    current.fpu.save();
    select_next(batch)
}

unsafe fn process_call(batch: *mut Batch, index: usize, nr: u64, a: u64, b: u64, c: u64) -> i64 {
    if (*batch).programs[index].i386.is_some(){return i386::call(&mut (*batch).programs[index],nr,a,b,c);}
    match nr {
        1004 => spawn(batch, index, a, b, c).unwrap_or_else(|e| e),
        61 => wait_child(batch, index, a, b, c).unwrap_or_else(|e| e),
        _ => {
            ACTIVE.store(&mut (*batch).programs[index], Ordering::Release);
            crate::syscall::dispatch(nr, a, b, c)
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn switch_user_syscall(context: *const Context) -> *const Context {
    let batch=BATCH.load(Ordering::Acquire);
    // The i386 address range can overlap the physical framebuffer. Use kernel
    // mappings for I/O and transfer user data through validated physical pages.
    Cr3::write((*batch).kernel_root,Cr3::read().1);
    let index = (*batch).current;
    {
        let program = &mut (*batch).programs[index];
        program.context = *context;
        program.fpu.save();
    }
    let registers = (*context).registers;
    let compat=(*batch).programs[index].i386.is_some();
    let (nr,a,b,c)=if compat{(registers[14]as u32 as u64,registers[13]as u32 as u64,registers[12]as u32 as u64,registers[11]as u32 as u64)}else{(registers[14],registers[8],registers[9],registers[11])};
    if (compat&&nr==0)||(!compat&&nr==60)||((*batch).programs[index].linux.enabled&&nr==231) {
        terminate(batch, index, (a & 255) as i64);
    } else {
        let result = if (*batch).programs[index].linux.enabled {
            linux::call(batch, index, nr, a, b, c)
        } else { process_call(batch, index, nr, a, b, c) };
        let program = &mut (*batch).programs[index];
        if result == -11 && ((compat&&matches!(nr,1|2|6))||(!compat&&matches!(nr,0|1|61|7|270|35|230))) {
            program.blocked = Some((nr, a, b, c));
        } else {
            program.context.registers[14] = result as u64;
        }
    }
    select_next(batch)
}

unsafe fn terminate(batch: *mut Batch, index: usize, status: i64) {
    let pid = (*batch).programs[index].pid;
    finish(&mut (*batch).programs[index], status);
    let adopter = if pid != 1
        && (*batch)
            .programs
            .iter()
            .any(|p| p.pid == 1 && p.status.is_none())
    {
        1
    } else {
        0
    };
    for p in &mut (*batch).programs {
        if p.parent == pid {
            p.parent = adopter;
        }
    }
}

unsafe fn spawn(
    batch: *mut Batch,
    index: usize,
    path: u64,
    mappings: u64,
    count: u64,
) -> Result<i64, i64> {
    if count > 64 {
        return Err(-22);
    }
    let parent = &(*batch).programs[index];
    let name = parent.path(path)?;
    let mut inherited: [Option<Arc<Handle>>; 64] = core::array::from_fn(|i| {
        if i < 3 {
            parent.descriptors[i].clone()
        } else {
            None
        }
    });
    let mut seen = 0u64;
    for n in 0..count {
        let mut pair = [0; 8];
        parent
            .read_user(mappings.checked_add(n * 8).ok_or(-14i64)?, &mut pair)
            .map_err(|_| -14i64)?;
        let from = u32::from_le_bytes(pair[..4].try_into().unwrap()) as u64;
        let to = u32::from_le_bytes(pair[4..].try_into().unwrap()) as usize;
        if to >= 64 || seen & (1 << to) != 0 {
            return Err(-22);
        }
        seen |= 1 << to;
        inherited[to] = Some(parent.descriptor(from)?);
    }
    let parent_pid = parent.pid;
    let parent_cwd = parent.linux.cwd.clone();
    let slot = (*batch).programs.iter().position(|p| p.reaped);
    if slot.is_none() && (*batch).programs.len() >= 64 {
        return Err(-11);
    }
    let bytes = {
        let mut vfs = crate::fs::VFS.lock();
        let stat = vfs
            .stat(&name)
            .map_err(|e| crate::syscall::fs_err(e) as i64)?;
        if stat.is_dir() || stat.size > 64 * 1024 * 1024 {
            return Err(-8);
        }
        vfs.read_file(&name)
            .map_err(|e| crate::syscall::fs_err(e) as i64)?
    };
    let mut child =
        load_program(&bytes).map_err(|e| if e == "out of memory" { -12i64 } else { -8i64 })?;
    child.pid = (*batch).next_pid;
    child.parent = parent_pid;
    child.linux.cwd = parent_cwd;
    child.descriptors = inherited;
    let pid = child.pid;
    (*batch).next_pid = pid.checked_add(1).ok_or(-11i64)?;
    if let Some(slot) = slot {
        (*batch).programs[slot] = child;
    } else {
        (*batch).programs.push(child);
    }
    Ok(pid as i64)
}

unsafe fn wait_child(
    batch: *mut Batch,
    index: usize,
    pid: u64,
    status: u64,
    flags: u64,
) -> Result<i64, i64> {
    if flags > 1 || (pid != u64::MAX && pid == 0) {
        return Err(-22);
    }
    let parent = &(*batch).programs[index];
    if status != 0 {
        parent.validate_write(status, 8)?;
    }
    let parent_pid = parent.pid;
    let mut found = false;
    let mut zombie = None;
    for (i, p) in (*batch).programs.iter().enumerate() {
        if !p.reaped && p.parent == parent_pid && (pid == u64::MAX || p.pid == pid) {
            found = true;
            if p.status.is_some() {
                zombie = Some(i);
                break;
            }
        }
    }
    if let Some(child) = zombie {
        let result = (*batch).programs[child].pid;
        let value = (*batch).programs[child].status.unwrap();
        if status != 0 {
            (*batch).programs[index].write_user(status, &value.to_le_bytes())?;
        }
        // Switch off a zombie's root before releasing its page tables.
        Cr3::write((*batch).kernel_root, Cr3::read().1);
        let p = &mut (*batch).programs[child];
        p.reaped = true;
        p.pages.clear();
        for frame in p.frames.0.drain(..) {
            phys::deallocate_frame(frame);
        }
        Ok(result as i64)
    } else if !found {
        Err(-10)
    } else if flags == 1 {
        Ok(0)
    } else {
        Err(-11)
    }
}

pub fn abort_on_fault(frame: &InterruptStackFrame, vector: i64) {
    if frame.code_segment.rpl() == PrivilegeLevel::Ring3
        && !ACTIVE.load(Ordering::Acquire).is_null()
    {
        unsafe{Cr3::write((*BATCH.load(Ordering::Acquire)).kernel_root,Cr3::read().1);}
        log::error!("user fault vector={} rip={:#x} addr={:?}", vector, frame.instruction_pointer.as_u64(), x86_64::registers::control::Cr2::read());
        exit(-vector - 1);
    }
}

/// The synchronous launcher owns Program for the duration of ring-3 execution.
/// Interrupts are disabled during kernel execution. This BSP-only state
/// must become per-CPU before enabling SMP user execution.
pub(crate) fn with_active_program(f: impl FnOnce(&mut Program) -> i64) -> i64 {
    let active = ACTIVE.load(Ordering::Acquire);
    if active.is_null() {
        return -38;
    }
    unsafe { f(&mut *active) }
}

pub(crate) fn exit(status: i64) -> ! {
    unsafe {
        let batch = BATCH.load(Ordering::Acquire);
        if batch.is_null() {
            leave_user(status);
        }
        terminate(batch, (*batch).current, status);
        resume_user(select_next(batch));
    }
}
