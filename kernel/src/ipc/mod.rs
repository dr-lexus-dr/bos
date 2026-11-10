//! IPC: pipes, message queues, shared memory stubs
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use spin::Mutex;

static PIPES: Mutex<BTreeMap<u64, VecDeque<u8>>> = Mutex::new(BTreeMap::new());
static NEXT_ID: Mutex<u64> = Mutex::new(1);

pub fn init() {}
pub fn pipe_new() -> u64 {
    let mut id = NEXT_ID.lock(); let v = *id; *id += 1;
    PIPES.lock().insert(v, VecDeque::new()); v
}
pub fn pipe_write(id: u64, data: &[u8]) -> usize {
    if let Some(q) = PIPES.lock().get_mut(&id) { q.extend(data.iter()); data.len() } else { 0 }
}
pub fn pipe_read(id: u64, buf: &mut [u8]) -> usize {
    if let Some(q) = PIPES.lock().get_mut(&id) {
        let n = q.len().min(buf.len());
        for b in buf.iter_mut().take(n) { *b = q.pop_front().unwrap_or(0); }
        n
    } else { 0 }
}
pub fn shm_create(_size: usize) -> u64 { 0 }
