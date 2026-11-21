//! Detached i386 tasks are scheduled between shell/desktop input polls.
//! Each task retains its own address space, terminal input and output.
use super::*;
use alloc::{format, string::String};
use core::sync::atomic::AtomicU64;
use spin::Mutex;
struct Job {
    batch: Batch,
    name: String,
    output: String,
}
static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static PID: AtomicU64 = AtomicU64::new(100);
static NEXT_POLL: AtomicU64 = AtomicU64::new(0);
pub fn start(bytes: &[u8], args: &[&str]) -> Result<u64, &'static str> {
    if crate::arch::x86_64::apic::timer_frequency() == 0 {
        return Err("timer unavailable");
    }
    let mut p = i386::load(bytes, args)?;
    let pid = PID.fetch_add(1, Ordering::Relaxed);
    p.pid = pid;
    let mut jobs = JOBS.lock();
    if jobs.len() >= 16 {
        if let Some(i) = jobs
            .iter()
            .position(|j| j.batch.programs[0].status.is_some())
        {
            jobs.remove(i);
        } else {
            return Err("too many jobs");
        }
    }
    let batch = Batch {
        programs: alloc::vec![p],
        current: 0,
        next_pid: pid + 1,
        native_init: false,
        kernel_root: Cr3::read().0,
        input: [0; 256],
        input_head: 0,
        input_len: 0,
        slice_end: Some(0),
    };
    jobs.push(Job {
        batch,
        name: args.first().copied().unwrap_or("program").into(),
        output: String::new(),
    });
    Ok(pid)
}
pub fn poll() {
    if !BATCH.load(Ordering::Acquire).is_null() {
        return;
    }
    let now = crate::boottime::elapsed_ms();
    if now < NEXT_POLL.load(Ordering::Relaxed) {
        return;
    }
    NEXT_POLL.store(now + 18, Ordering::Relaxed);
    let Some(mut jobs) = JOBS.try_lock() else {
        return;
    };
    for job in jobs.iter_mut() {
        let b = &mut job.batch;
        if b.programs[0].status.is_some() {
            continue;
        }
        let out = console::capture(|| {
            if let Some((nr, a, x, c)) = b.programs[0].blocked {
                // Retry on the kernel stack before returning to the saved user instruction.
                BATCH.store(b as *mut Batch, Ordering::Release);
                let val = i386::call(&mut b.programs[0], nr, a, x, c);
                BATCH.store(ptr::null_mut(), Ordering::Release);
                if val == -11 {
                    return;
                }
                b.programs[0].context.registers[14] = val as u64;
                b.programs[0].blocked = None;
            }
            b.slice_end = Some(crate::boottime::elapsed_ms() + 6);
            run_batch(b);
        });
        job.output.push_str(&out);
        if job.output.len() > 32768 {
            let mut n = job.output.len() - 24576;
            while !job.output.is_char_boundary(n) {
                n += 1;
            }
            job.output.drain(..n);
        }
    }
}
pub fn input(pid: u64, line: &str) -> bool {
    let mut jobs = JOBS.lock();
    let Some(j) = jobs.iter_mut().find(|j| j.batch.programs[0].pid == pid) else {
        return false;
    };
    let b = &mut j.batch;
    if line.len().saturating_add(1) > b.input.len() - b.input_len {
        return false;
    }
    for c in line.bytes().chain(core::iter::once(b'\n')) {
        b.input[(b.input_head + b.input_len) % 256] = c;
        b.input_len += 1;
    }
    true
}
pub fn take(pid: u64) -> Option<(String, Option<i64>)> {
    let mut jobs = JOBS.lock();
    let j = jobs.iter_mut().find(|j| j.batch.programs[0].pid == pid)?;
    Some((core::mem::take(&mut j.output), j.batch.programs[0].status))
}
pub fn kill(pid: u64) -> bool {
    let mut jobs = JOBS.lock();
    if let Some(j) = jobs
        .iter_mut()
        .find(|j| j.batch.programs[0].pid == pid && j.batch.programs[0].status.is_none())
    {
        finish(&mut j.batch.programs[0], 130);
        true
    } else {
        false
    }
}
pub fn list() -> String {
    let mut out = String::from("PID   STATE     COMMAND\n1     running   bSH\n");
    for j in JOBS.lock().iter() {
        let p = &j.batch.programs[0];
        let state = if p.status.is_some() {
            "done"
        } else if p.blocked.is_some() {
            "waiting"
        } else {
            "running"
        };
        out.push_str(&format!("{:<5} {:<9} {}\n", p.pid, state, j.name));
    }
    out
}
