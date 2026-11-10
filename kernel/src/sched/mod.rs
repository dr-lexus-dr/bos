//! Scheduler

use core::sync::atomic::{AtomicU64, AtomicU32, AtomicBool, Ordering};
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use spin::{Mutex, RwLock};
use lazy_static::lazy_static;
use log::{info, trace, warn};
use crate::proc::{Process, Thread, ThreadState, Priority, CpuContext};
use crate::arch::x86_64::{enable_interrupts, disable_interrupts, halt};
use crate::arch::x86_64::cpu::{current_cpu_id, current_task, set_current_task, kernel_stack, current};
use crate::arch::x86_64::apic::{send_ipi_all_excl_self, IPI_RESCHEDULE};

/// Time slice in nanoseconds (10ms default)
const DEFAULT_TIME_SLICE: u64 = 10_000_000;

/// Scheduler state per CPU
struct CpuScheduler {
    runqueues: [Vec<Arc<Mutex<Thread>>>; 64], // 64 priority levels
    current_thread: Option<Arc<Mutex<Thread>>>,
    idle_thread: Arc<Mutex<Thread>>,
    need_reschedule: bool,
    tick_count: u64,
    context_switches: u64,
}

impl CpuScheduler {
    fn new() -> Self {
        Self {
            runqueues: core::array::from_fn(|_| Vec::new()),
            current_thread: None,
            idle_thread: Arc::new(Mutex::new(Thread::idle())),
            need_reschedule: false,
            tick_count: 0,
            context_switches: 0,
        }
    }
}

lazy_static! {
    static ref SCHEDULERS: RwLock<BTreeMap<u32, Arc<Mutex<CpuScheduler>>>> = RwLock::new(BTreeMap::new());
    static ref TICK_COUNT: AtomicU64 = AtomicU64::new(0);
    static ref SCHEDULER_INITIALIZED: AtomicBool = AtomicBool::new(false);
}

/// Initialize scheduler
pub fn init() {
    // Create BSP scheduler
    let cpu_id = current_cpu_id();
    create_scheduler(cpu_id);

    SCHEDULER_INITIALIZED.store(true, Ordering::SeqCst);
    info!("Scheduler initialized on CPU {}", cpu_id);
}

/// Initialize scheduler for AP
pub fn init_ap(cpu_id: u32) {
    create_scheduler(cpu_id);
    info!("Scheduler initialized on AP CPU {}", cpu_id);
}

fn create_scheduler(cpu_id: u32) {
    let mut sched = CpuScheduler::new();

    // Create idle thread for this CPU
    let idle_thread = Thread::idle_for_cpu(cpu_id);
    sched.idle_thread = Arc::new(Mutex::new(idle_thread));

    SCHEDULERS.write().insert(cpu_id, Arc::new(Mutex::new(sched)));
}

/// Get scheduler for current CPU
fn current_scheduler() -> Option<Arc<Mutex<CpuScheduler>>> {
    let cpu_id = current_cpu_id();
    SCHEDULERS.read().get(&cpu_id).cloned()
}

/// Get scheduler for specific CPU
pub fn get_scheduler(cpu_id: u32) -> Option<Arc<Mutex<CpuScheduler>>> {
    SCHEDULERS.read().get(&cpu_id).cloned()
}

/// Create a new thread
pub fn create_thread(
    process: &mut Process,
    entry_point: u64,
    arg: u64,
    priority: Priority,
) -> Result<u64, &'static str> {
    let tid = crate::proc::NEXT_PID.fetch_add(1, Ordering::SeqCst); // Reuse PID generator for TID

    // Allocate kernel stack
    let stack_size = 16 * 1024; // 16 KB
    let stack_base = crate::mm::phys::allocate_frames(stack_size / 4096)
        .ok_or("Failed to allocate thread stack")?
        .start_addr + stack_size as u64;

    let mut thread = Thread {
        tid,
        pid: process.pid,
        state: ThreadState::Created,
        priority,
        stack_base,
        stack_size,
        entry_point,
        arg,
        cpu_context: CpuContext::default(),
        fpu_state: None,
        tls_base: 0,
        errno: 0,
        sleep_until: 0,
        wait_queue: None,
        affinity: (1 << crate::arch::x86_64::cpu::cpu_count()) - 1, // All CPUs
        time_slice: DEFAULT_TIME_SLICE,
        used_time: 0,
    };

    // Set up initial CPU context
    {
        let cpu_ctx = &mut thread.cpu_context;
        cpu_ctx.rip = entry_point;
        cpu_ctx.rsp = stack_base;
        cpu_ctx.rflags = 0x202; // IF bit set
        cpu_ctx.cs = 0x10; // User code segment (will be set properly)
        cpu_ctx.ss = 0x18; // User data segment
        cpu_ctx.rdi = arg; // First argument in RDI
    }

    process.threads.push(thread);
    Ok(tid)
}

/// Add thread to runqueue
pub fn enqueue_thread(thread: Arc<Mutex<Thread>>) {
    let priority = {
        let mut thread_guard = thread.lock();
        thread_guard.state = ThreadState::Ready;
        thread_guard.priority as usize
    };
    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        sched.runqueues[priority].push(thread);
        sched.need_reschedule = true;
    }
}

/// Remove thread from runqueue
pub fn dequeue_thread(tid: u64) -> Option<Arc<Mutex<Thread>>> {
    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        for queue in &mut sched.runqueues {
            if let Some(pos) = queue.iter().position(|t| t.lock().tid == tid) {
                let thread = queue.remove(pos);
                return Some(thread);
            }
        }
    }
    None
}

/// Schedule next thread
pub fn schedule() -> Option<Arc<Mutex<Thread>>> {
    let scheduler = current_scheduler()?;
    let mut sched = scheduler.lock();
    schedule_locked(&mut sched)
}

fn schedule_locked(sched: &mut CpuScheduler) -> Option<Arc<Mutex<Thread>>> {
    // Find highest priority ready thread
    for priority in (0..64).rev() {
        if let Some(thread) = sched.runqueues[priority].pop() {
            let ready = {
                let mut thread_guard = thread.lock();
                if thread_guard.state == ThreadState::Ready {
                    thread_guard.state = ThreadState::Running;
                    true
                } else {
                    false
                }
            };
            if ready {
                sched.current_thread = Some(thread.clone());
                sched.context_switches += 1;
                return Some(thread);
            }
            // Thread not ready anymore, put back
            sched.runqueues[priority].push(thread);
        }
    }

    // No ready threads, return idle thread
    let idle = sched.idle_thread.clone();
    idle.lock().state = ThreadState::Running;
    sched.current_thread = Some(idle.clone());
    Some(idle)
}

/// Context switch to next thread
pub fn context_switch() {
    disable_interrupts();

    let scheduler = match current_scheduler() {
        Some(s) => s,
        None => {
            enable_interrupts();
            return;
        }
    };

    let next = {
        let mut sched = scheduler.lock();

        // Save current thread context
        if let Some(current) = sched.current_thread.take() {
            let (running, prio) = {
                let mut current_guard = current.lock();
                let was_running = current_guard.state == ThreadState::Running;
                if was_running {
                    // Save CPU context (done in assembly)
                    current_guard.state = ThreadState::Ready;
                }
                (was_running, current_guard.priority as usize)
            };
            if running {
                sched.runqueues[prio].push(current);
            }
        }

        // Schedule next thread (same lock, no re-lock)
        let next = schedule_locked(&mut sched);
        if let Some(ref n) = next {
            n.lock().state = ThreadState::Running;
            sched.current_thread = Some(n.clone());
        }
        next
    };

    // Switch to new thread (assembly stub)
    if let Some(next) = next {
        let guard = next.lock();
        unsafe { switch_to_thread(&guard) };
    }

    enable_interrupts();
}

/// Switch to specific thread (stub: real switch happens in IRQ return path)
unsafe fn switch_to_thread(_thread: &Thread) {
    // Intentionally empty: cooperative stub until full asm switch lands.
}

/// Yield current thread
pub fn yield_now() {
    // Trigger reschedule
    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        sched.need_reschedule = true;
    }

    // Trigger context switch via interrupt
    unsafe {
        core::arch::asm!("int 0x20", options(nomem, nostack)); // Timer interrupt
    }
}

/// Timer tick handler
pub fn timer_tick() {
    TICK_COUNT.fetch_add(1, Ordering::SeqCst);

    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        sched.tick_count += 1;

        // Update current thread time
        let expired = if let Some(current) = sched.current_thread.clone() {
            let mut current_guard = current.lock();
            current_guard.used_time += 1;

            // Check if time slice expired
            if current_guard.used_time >= current_guard.time_slice {
                current_guard.used_time = 0;
                true
            } else {
                false
            }
        } else {
            false
        };
        if expired {
            sched.need_reschedule = true;
        }
    }
}

/// Reschedule IPI handler
pub fn reschedule_ipi() {
    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        sched.need_reschedule = true;
    }
}

/// Force reschedule on current CPU
pub fn reschedule() {
    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        sched.need_reschedule = true;
    }
}

/// Check if reschedule is needed
pub fn need_reschedule() -> bool {
    if let Some(scheduler) = current_scheduler() {
        scheduler.lock().need_reschedule
    } else {
        false
    }
}

/// Set thread affinity
pub fn set_affinity(tid: u64, affinity: u64) -> Result<(), &'static str> {
    // Find thread and update affinity
    Ok(())
}

/// Get current thread
pub fn current_thread() -> Option<Arc<Mutex<Thread>>> {
    current_scheduler()?.lock().current_thread.clone()
}

/// Get tick count
pub fn tick_count() -> u64 {
    TICK_COUNT.load(Ordering::SeqCst)
}

/// Sleep current thread for specified nanoseconds
pub fn sleep_ns(ns: u64) {
    if let Some(current) = current_thread() {
        let mut thread = current.lock();
        let wake_time = crate::arch::x86_64::rdtsc() + ns_to_cycles(ns);
        thread.sleep_until = wake_time;
        thread.state = ThreadState::Sleeping;
        drop(thread);
        yield_now();
    }
}

/// Convert nanoseconds to TSC cycles
fn ns_to_cycles(ns: u64) -> u64 {
    // Approximate: assume 3 GHz CPU
    ns * 3
}

/// Wake up sleeping threads
pub fn wake_sleeping_threads() {
    let now = crate::arch::x86_64::rdtsc();

    if let Some(scheduler) = current_scheduler() {
        let mut sched = scheduler.lock();
        for queue in &mut sched.runqueues {
            queue.retain(|thread| {
                let mut t = thread.lock();
                if t.state == ThreadState::Sleeping && t.sleep_until <= now {
                    t.state = ThreadState::Ready;
                    false // Remove from sleeping, will be re-added to runqueue
                } else {
                    true
                }
            });
        }
    }
}

/// AP scheduler main loop
pub fn ap_main(cpu_id: u32) -> ! {
    init_ap(cpu_id);
    enable_interrupts();

    loop {
        // Check for work
        if need_reschedule() {
            context_switch();
        } else {
            // Run idle thread
            halt();
        }
    }
}

/// Scheduler statistics
pub fn stats() -> SchedulerStats {
    let cpu_id = current_cpu_id();
    if let Some(scheduler) = get_scheduler(cpu_id) {
        let sched = scheduler.lock();
        SchedulerStats {
            cpu_id,
            tick_count: sched.tick_count,
            context_switches: sched.context_switches,
            runqueue_lengths: sched.runqueues.iter().map(|q| q.len()).collect(),
        }
    } else {
        SchedulerStats::default()
    }
}

#[derive(Debug, Default)]
pub struct SchedulerStats {
    pub cpu_id: u32,
    pub tick_count: u64,
    pub context_switches: u64,
    pub runqueue_lengths: Vec<usize>,
}

/// Enqueue main thread of a process by pid (used by init)
pub fn enqueue_thread_of_process(pid: u64) {
    if let Some(p) = crate::proc::get_process(pid) {
        let proc = p.lock();
        if let Some(t) = proc.threads.first() {
            let tid = t.tid;
            drop(proc);
            // Rebuild Arc<Mutex<Thread>> wrapper: find in process and enqueue a clone handle
            // Simplified: create a sched thread entry pointing at same tid via process lookup
            if let Some(sched) = current_scheduler() {
                let mut s = sched.lock();
                // push a placeholder idle-like thread with correct tid so scheduler has work
                let th = alloc::sync::Arc::new(spin::Mutex::new(crate::proc::Thread {
                    tid,
                    pid,
                    state: crate::proc::ThreadState::Ready,
                    priority: crate::proc::Priority::Normal,
                    stack_base: 0, stack_size: 0, entry_point: 0, arg: 0,
                    cpu_context: crate::proc::CpuContext::default(),
                    fpu_state: None, tls_base: 0, errno: 0, sleep_until: 0,
                    wait_queue: None, affinity: 0xFFFF_FFFF_FFFF_FFFF,
                    time_slice: DEFAULT_TIME_SLICE, used_time: 0,
                }));
                s.runqueues[crate::proc::Priority::Normal as usize].push(th);
                s.need_reschedule = true;
            }
        }
    }
}

/// Called from kernel main loop: switch if needed
pub fn maybe_reschedule() {
    wake_sleeping_threads();
    if need_reschedule() {
        context_switch();
    }
}