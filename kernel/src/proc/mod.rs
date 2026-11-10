//! Process Management

use core::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::{String, ToString};
use alloc::format;
use alloc::sync::Arc;
use alloc::collections::BTreeMap;
use spin::{Mutex, RwLock};
use lazy_static::lazy_static;
use log::{info, trace, warn, error};
use crate::mm::paging::{AddressSpace, VirtAddr, PhysAddr, PageTableFlags};

/// Maximum number of processes
const MAX_PROCESSES: u64 = 1 << 20; // ~1 million

/// Process ID generator
pub static NEXT_PID: AtomicU64 = AtomicU64::new(1);

/// Process state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Created,
    Ready,
    Running,
    Blocked,
    Sleeping,
    Zombie,
    Dead,
}

/// Process priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Idle = 0,
    Low = 10,
    Normal = 20,
    High = 30,
    Realtime = 40,
    Kernel = 50,
}

/// Thread state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Blocked,
    Sleeping,
    Dead,
}

/// Process structure
pub struct Process {
    pub pid: u64,
    pub ppid: u64,
    pub name: String,
    pub state: ProcessState,
    pub priority: Priority,
    pub address_space: AddressSpace,
    pub threads: Vec<Thread>,
    pub main_thread: u64,
    pub exit_code: i32,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub fd_table: FileDescriptorTable,
    pub memory_regions: Vec<MemoryRegion>,
    pub start_time: u64,
    pub cpu_time: u64,
    pub parent: Option<Arc<Mutex<Process>>>,
    pub children: Vec<Arc<Mutex<Process>>>,
    pub signal_handlers: [SignalHandler; 32],
    pub pending_signals: u32,
    pub signal_mask: u32,
}

/// Thread structure
pub struct Thread {
    pub tid: u64,
    pub pid: u64,
    pub state: ThreadState,
    pub priority: Priority,
    pub stack_base: u64,
    pub stack_size: usize,
    pub entry_point: u64,
    pub arg: u64,
    pub cpu_context: CpuContext,
    pub fpu_state: Option<FpuState>,
    pub tls_base: u64,
    pub errno: i32,
    pub sleep_until: u64,
    pub wait_queue: Option<u64>,
    pub affinity: u64, // CPU affinity mask
    pub time_slice: u64,
    pub used_time: u64,
}

/// CPU context for context switching
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct CpuContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub cs: u64,
    pub ss: u64,
    pub fs_base: u64,
    pub gs_base: u64,
}

/// FPU state (FXSAVE area)
#[repr(C, align(16))]
pub struct FpuState {
    pub data: [u8; 512],
}

impl Default for FpuState {
    fn default() -> Self {
        Self { data: [0; 512] }
    }
}

/// Memory region for process
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start: VirtAddr,
    pub end: VirtAddr,
    pub flags: PageTableFlags,
    pub file_backed: bool,
    pub file_offset: u64,
    pub file_path: Option<String>,
}

/// File descriptor
#[derive(Debug, Clone)]
pub struct FileDescriptor {
    pub flags: u32,
    pub offset: u64,
    pub inode: u64,
    pub filesystem: String,
    pub path: String,
}

/// File descriptor table
#[derive(Debug, Clone)]
pub struct FileDescriptorTable {
    entries: Vec<Option<FileDescriptor>>,
    next_fd: u32,
}

impl FileDescriptorTable {
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(64),
            next_fd: 0,
        }
    }

    pub fn allocate(&mut self, fd: FileDescriptor) -> u32 {
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.is_none() {
                *entry = Some(fd);
                return i as u32;
            }
        }
        let fd_num = self.entries.len() as u32;
        self.entries.push(Some(fd));
        fd_num
    }

    pub fn get(&self, fd: u32) -> Option<&FileDescriptor> {
        self.entries.get(fd as usize).and_then(|e| e.as_ref())
    }

    pub fn get_mut(&mut self, fd: u32) -> Option<&mut FileDescriptor> {
        self.entries.get_mut(fd as usize).and_then(|e| e.as_mut())
    }

    pub fn close(&mut self, fd: u32) -> Option<FileDescriptor> {
        if fd < self.entries.len() as u32 {
            self.entries[fd as usize].take()
        } else {
            None
        }
    }
}

/// Signal handler
#[derive(Debug, Clone, Copy)]
pub struct SignalHandler {
    pub handler: u64,
    pub flags: u32,
    pub mask: u32,
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self { handler: 0, flags: 0, mask: 0 }
    }
}

/// Process table
lazy_static! {
    static ref PROCESS_TABLE: RwLock<BTreeMap<u64, Arc<Mutex<Process>>>> = RwLock::new(BTreeMap::new());
    static ref CURRENT_PROCESS: AtomicU64 = AtomicU64::new(0);
}

/// Create a new process
pub fn create_process(
    name: &str,
    entry_point: u64,
    arg: u64,
    priority: Priority,
    address_space: Option<AddressSpace>,
) -> Result<Arc<Mutex<Process>>, &'static str> {
    let pid = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    if pid >= MAX_PROCESSES {
        return Err("Process limit reached");
    }
    let address_space = address_space.unwrap_or_else(|| crate::mm::paging::create_address_space());

    let process = Process {
        pid,
        ppid: 0,
        name: name.to_string(),
        state: ProcessState::Created,
        priority,
        address_space,
        threads: Vec::new(),
        main_thread: 0,
        exit_code: 0,
        cwd: "/".to_string(),
        env: BTreeMap::new(),
        fd_table: FileDescriptorTable::new(),
        memory_regions: Vec::new(),
        start_time: crate::arch::x86_64::rdtsc(),
        cpu_time: 0,
        parent: None,
        children: Vec::new(),
        signal_handlers: [SignalHandler::default(); 32],
        pending_signals: 0,
        signal_mask: 0,
    };

    let process = Arc::new(Mutex::new(process));

    // Create main thread
    let mut proc = process.lock();
    let tid = crate::sched::create_thread(&mut proc, entry_point, arg, priority)?;
    proc.main_thread = tid;

    proc.state = ProcessState::Ready;
    drop(proc);

    // Add to process table
    PROCESS_TABLE.write().insert(pid, process.clone());

    info!("Created process {} ({}) with PID {}", name, tid, pid);
    Ok(process)
}

/// Get process by PID
pub fn get_process(pid: u64) -> Option<Arc<Mutex<Process>>> {
    PROCESS_TABLE.read().get(&pid).cloned()
}

/// Get current process
pub fn current_process() -> Option<Arc<Mutex<Process>>> {
    let pid = CURRENT_PROCESS.load(Ordering::SeqCst);
    if pid == 0 {
        None
    } else {
        get_process(pid)
    }
}

/// Set current process
pub fn set_current_process(process: Option<Arc<Mutex<Process>>>) {
    CURRENT_PROCESS.store(process.map(|p| p.lock().pid).unwrap_or(0), Ordering::SeqCst);
}

/// Exit current process
pub fn exit_current(exit_code: i32) -> ! {
    if let Some(process) = current_process() {
        let mut proc = process.lock();
        proc.exit_code = exit_code;
        proc.state = ProcessState::Zombie;

        // Close all file descriptors
        for i in 0..proc.fd_table.entries.len() {
            proc.fd_table.close(i as u32);
        }

        // Notify parent
        if let Some(parent) = &proc.parent {
            // Send SIGCHLD to parent
        }

        // Switch to scheduler
        crate::sched::yield_now();
    }
    panic!("exit_current called without current process");
}

/// Wait for process
pub fn wait_pid(pid: u64) -> Option<i32> {
    // Implementation would wait for specific child
    None
}

/// Fork current process
pub fn fork() -> Result<u64, &'static str> {
    let current = current_process().ok_or("No current process")?;
    let mut proc = current.lock();

    // Create child process with copied address space
    let child_as = proc.address_space.clone();
    let child = create_process(
        &format!("{} (fork)", proc.name),
        proc.threads[0].entry_point,
        proc.threads[0].arg,
        proc.priority,
        Some(child_as),
    )?;

    // Set parent
    child.lock().parent = Some(current.clone());
    proc.children.push(child.clone());

    // Copy file descriptors
    {
        let mut child_proc = child.lock();
        child_proc.fd_table = proc.fd_table.clone();
        // Increment reference counts on files
    }

    // Copy memory regions
    child.lock().memory_regions = proc.memory_regions.clone();

    // Return child PID to parent, 0 to child
    // This is handled in the syscall return path
    let child_pid = child.lock().pid;
    Ok(child_pid)
}

/// Exec a new program
pub fn exec(path: &str, args: &[String], env: &[String]) -> Result<(), &'static str> {
    let current = current_process().ok_or("No current process")?;
    let mut proc = current.lock();

    // Load executable
    let entry_point = load_executable(path, &mut proc.address_space)?;

    // Reset process state
    proc.name = path.to_string();
    proc.threads.clear();
    proc.memory_regions.clear();
    proc.main_thread = 0;

    // Create new main thread
    let prio = proc.priority;
    let tid = crate::sched::create_thread(&mut proc, entry_point, 0, prio)?;
    proc.main_thread = tid;

    // Set up stack with arguments
    setup_stack_for_exec(&mut proc, args, env)?;

    Ok(())
}

/// Load executable (ELF, PE, Mach-O)
fn load_executable(path: &str, address_space: &mut AddressSpace) -> Result<u64, &'static str> {
    // Try different loaders
    if path.ends_with(".elf") || is_elf(path) {
        crate::compat::elf::load_elf(path, address_space)
    } else if path.ends_with(".exe") || is_pe(path) {
        crate::compat::win32::load_pe(path, address_space)
    } else if is_macho(path) {
        crate::compat::macho::load_macho(path, address_space)
    } else {
        // Try all
        crate::compat::elf::load_elf(path, address_space)
            .or_else(|_| crate::compat::win32::load_pe(path, address_space))
            .or_else(|_| crate::compat::macho::load_macho(path, address_space))
    }
}

fn is_elf(path: &str) -> bool {
    // Check magic bytes
    false
}

fn is_pe(path: &str) -> bool {
    false
}

fn is_macho(path: &str) -> bool {
    false
}

fn setup_stack_for_exec(proc: &mut Process, args: &[String], env: &[String]) -> Result<(), &'static str> {
    // Set up user stack with argc, argv, envp
    Ok(())
}

/// Send signal to process
pub fn send_signal(pid: u64, signal: u32) -> Result<(), &'static str> {
    let process = get_process(pid).ok_or("Process not found")?;
    let mut proc = process.lock();

    if signal < 32 {
        proc.pending_signals |= 1 << signal;
    }

    // Wake up process if sleeping
    if proc.state == ProcessState::Sleeping {
        proc.state = ProcessState::Ready;
    }

    Ok(())
}

/// Handle signals for current process
pub fn handle_signals() {
    if let Some(process) = current_process() {
        let mut proc = process.lock();
        let pending = proc.pending_signals & !proc.signal_mask;

        for sig in 0..32 {
            if pending & (1 << sig) != 0 {
                let handler = proc.signal_handlers[sig];
                if handler.handler != 0 {
                    // Call signal handler
                    // This requires setting up a signal trampoline on user stack
                    proc.pending_signals &= !(1 << sig);
                } else if sig == 8 { // SIGKILL
                    exit_current(-1);
                }
            }
        }
    }
}

/// Process iterator
pub fn iter_processes() -> impl Iterator<Item = Arc<Mutex<Process>>> {
    PROCESS_TABLE.read().values().cloned().collect::<Vec<_>>().into_iter()
}

/// Get process count
pub fn process_count() -> usize {
    PROCESS_TABLE.read().len()
}

/// Initialize process management
pub fn init() {
    // Create init process (PID 1)
    let init_process = create_process(
        "init",
        0, // Will be set when init binary loads
        0,
        Priority::Normal,
        None,
    ).expect("Failed to create init process");

    set_current_process(Some(init_process.clone()));
    info!("Process management initialized");
}

pub fn current_pid() -> Option<u64> {
    Some(CURRENT_PROCESS.load(Ordering::SeqCst)).filter(|p| *p != 0)
}

pub fn handle_user_page_fault(
    addr: x86_64::VirtAddr,
    _err: x86_64::structures::idt::PageFaultErrorCode,
) -> Result<(), &'static str> {
    let _ = addr;
    Err("user page fault: unhandled")
}

impl Thread {
    pub fn idle() -> Self {
        Self {
            tid: 0, pid: 0, state: ThreadState::Ready, priority: Priority::Idle,
            stack_base: 0, stack_size: 0, entry_point: 0, arg: 0,
            cpu_context: CpuContext::default(), fpu_state: None, tls_base: 0,
            errno: 0, sleep_until: 0, wait_queue: None, affinity: 0xFFFF_FFFF_FFFF_FFFF,
            time_slice: 10_000_000, used_time: 0,
        }
    }
    pub fn idle_for_cpu(cpu_id: u32) -> Self {
        let mut t = Self::idle();
        t.tid = 0x1_0000 + cpu_id as u64;
        t.affinity = 1 << cpu_id;
        t
    }
}