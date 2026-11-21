//! A BSP-only cooperative stack for an outstanding HTTP request. DNS, TCP and
//! TLS can wait for the wire without stopping mouse input or the compositor.
use alloc::{boxed::Box, string::String, vec, vec::Vec};
use bos_web_runtime::Request;
use core::{
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
};
type Response = Result<(String, Vec<u8>), String>;
core::arch::global_asm!(
    r#"
.text
.global bos_web_swap
bos_web_swap:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rdi],rsp
    mov rsp,rsi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
"#
);
extern "C" {
    fn bos_web_swap(save: *mut usize, next: usize);
}
struct Context {
    sp: usize,
    caller: usize,
    stack: Box<[u128]>,
    request: Option<Request>,
    result: Option<Response>,
    deadline: u64,
    cancel: bool,
}
static CURRENT: AtomicPtr<Context> = AtomicPtr::new(ptr::null_mut());
static BUSY: AtomicBool = AtomicBool::new(false);
pub struct Transfer(Box<Context>);
impl Transfer {
    pub fn start(r: Request) -> Result<Self, Request> {
        if BUSY.swap(true, Ordering::AcqRel) {
            return Err(r);
        }
        let mut c = Box::new(Context {
            sp: 0,
            caller: 0,
            stack: vec![0u128; 16 * 1024].into_boxed_slice(),
            request: Some(r),
            result: None,
            deadline: 0,
            cancel: false,
        });
        let top = unsafe { c.stack.as_mut_ptr().add(c.stack.len()) as *mut usize };
        unsafe {
            let frame = top.sub(8);
            frame.add(6).write(worker as *const () as usize);
            c.sp = frame as usize;
        }
        Ok(Self(c))
    }
    fn resume(&mut self) {
        let c = ptr::from_mut(&mut *self.0);
        unsafe {
            (*c).deadline = crate::boottime::elapsed_ms() + 4;
            CURRENT.store(c, Ordering::Release);
            bos_web_swap(&raw mut (*c).caller, (*c).sp);
            CURRENT.store(ptr::null_mut(), Ordering::Release);
        }
    }
    pub fn step(&mut self) -> Option<Response> {
        self.resume();
        self.0.result.take()
    }
}
impl Drop for Transfer {
    fn drop(&mut self) {
        if self.0.sp != 0 {
            self.0.cancel = true;
            self.resume();
        }
        BUSY.store(false, Ordering::Release);
    }
}
extern "C" fn worker() -> ! {
    let c = CURRENT.load(Ordering::Acquire);
    unsafe {
        let request = (*c).request.take().unwrap();
        let result = super::engine::transfer(request);
        (*c).result = Some(result);
        (*c).sp = 0;
        let mut discarded = 0;
        bos_web_swap(&mut discarded, (*c).caller);
        core::hint::unreachable_unchecked()
    }
}
pub fn checkpoint() -> bool {
    let c = CURRENT.load(Ordering::Acquire);
    if c.is_null() {
        return false;
    }
    unsafe {
        if !(*c).cancel && crate::boottime::elapsed_ms() >= (*c).deadline {
            bos_web_swap(&raw mut (*c).sp, (*c).caller);
        }
        (*c).cancel
    }
}
