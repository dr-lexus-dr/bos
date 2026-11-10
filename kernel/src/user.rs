//! One synchronous ring-3 process, with private ELF mappings and a small ABI.

use alloc::vec::Vec;
use core::{
    arch::global_asm,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use x86_64::{
    registers::{
        control::{Cr0, Cr0Flags, Cr3},
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
const IMAGE_END: u64 = USER_BASE + 4 * 1024 * 1024;
const STACK_TOP: u64 = IMAGE_END + 5 * PAGE;
const STACK_BOTTOM: u64 = STACK_TOP - 4 * PAGE;

static ACTIVE: AtomicPtr<Program> = AtomicPtr::new(ptr::null_mut());

global_asm!(
    include_str!("user_entry.asm"),
    user_cs = const gdt::USER_CS64.0,
    user_ds = const gdt::USER_DS.0,
    options(att_syntax),
);

unsafe extern "C" {
    fn enter_user(entry: u64, stack: u64) -> i64;
    fn leave_user(status: i64) -> !;
    pub fn user_syscall();
}

struct Frames(Vec<PhysFrame>);

unsafe impl FrameAllocator<Size4KiB> for Frames {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = phys::allocate_frame()?;
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

struct Program {
    frames: Frames,
    root: PhysFrame,
    pages: Vec<(u64, PhysFrame)>,
}

impl Program {
    fn new() -> Result<Self, &'static str> {
        let mut frames = Frames(Vec::new());
        let root = frames.allocate_frame().ok_or("out of memory")?;
        let old = Cr3::read().0.start_address().as_u64() as *const PageTable;
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
        })
    }

    fn page(&mut self, address: u64, flags: Flags) -> Result<PhysFrame, &'static str> {
        if self.pages.iter().any(|(page, _)| *page == address) {
            return Err("overlapping ELF pages");
        }
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
                .map_err(|_| "could not map a user page")?
                .ignore();
        }
        self.pages.push((address, frame));
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

    fn read_user(&self, address: u64, output: &mut [u8]) -> Result<(), ()> {
        let mut address = address;
        let mut offset = 0;
        while offset < output.len() {
            let page = address & !(PAGE - 1);
            let (_, frame) = self
                .pages
                .iter()
                .find(|(base, _)| *base == page)
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

pub fn run(data: &[u8]) -> Result<i64, &'static str> {
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

    let (previous_root, flags) = Cr3::read();
    let cr0 = Cr0::read();
    let efer = Efer::read();
    ACTIVE.store(ptr::addr_of_mut!(program), Ordering::Release);
    let status = unsafe {
        // This ABI uses int 0x80 only. Deny user FPU access until context switching exists.
        Efer::update(|flags| flags.remove(EferFlags::SYSTEM_CALL_EXTENSIONS));
        Cr0::update(|flags| flags.insert(Cr0Flags::TASK_SWITCHED));
        Cr3::write(program.root, flags);
        let status = enter_user(executable.entry, STACK_TOP);
        Cr3::write(previous_root, flags);
        Cr0::write(cr0);
        Efer::write(efer);
        status
    };
    ACTIVE.store(ptr::null_mut(), Ordering::Release);
    Ok(status)
}

pub fn abort_on_fault(frame: &InterruptStackFrame, vector: i64) {
    if frame.code_segment.rpl() == PrivilegeLevel::Ring3
        && !ACTIVE.load(Ordering::Acquire).is_null()
    {
        unsafe {
            leave_user(-vector - 1);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn dispatch_user_syscall(number: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    let active = ACTIVE.load(Ordering::Acquire);
    if active.is_null() {
        return -38;
    }
    match number {
        1 => {
            if !matches!(arg0, 1 | 2) {
                return -9;
            }
            if arg2 > 4096 {
                return -22;
            }
            let mut buffer = [0; 4096];
            let output = &mut buffer[..arg2 as usize];
            if unsafe { (*active).read_user(arg1, output) }.is_err() {
                return -14;
            }
            let output = alloc::string::String::from_utf8_lossy(output);
            for part in output.split_inclusive('\n') {
                if let Some(line) = part.strip_suffix('\n') {
                    console::print(line.trim_end_matches('\r'));
                    console::print("\r\n");
                } else {
                    console::print(part);
                }
            }
            arg2 as i64
        }
        60 => unsafe { leave_user((arg0 & 0xff) as i64) },
        _ => -38,
    }
}
