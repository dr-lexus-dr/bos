//! Cooperative, capability-limited host for the bundled browser executable.
#![no_std]
extern crate alloc;
mod wasi;
use alloc::{
    collections::{BTreeSet, VecDeque},
    string::{String, ToString},
    vec::Vec,
};
use wasmi::{
    Caller, CompilationMode, Config, Engine, Error, ExternType, Func, Linker, Memory, Module,
    ResumableCall, Store, StoreLimits, StoreLimitsBuilder, Val,
};

pub trait Platform: Send + 'static {
    fn monotonic_ms() -> u64;
    fn epoch_ms() -> u64;
    fn random(dst: &mut [u8]) -> bool;
    fn log(message: &str);
}
pub struct Library {
    engine: Engine,
    module: Module,
}
impl Library {
    pub fn new(bytes: &[u8]) -> Result<Self, Error> {
        let mut cfg = Config::default();
        cfg.compilation_mode(CompilationMode::Lazy);
        let engine = Engine::new(&cfg);
        let module = Module::new(&engine, bytes)?;
        Ok(Self { engine, module })
    }
}
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub document_height: usize,
    pub scroll: usize,
    pub ready: bool,
    pub status: u16,
    pub url: String,
    pub title: String,
    pub pixels: Vec<u32>,
}
pub struct Request {
    pub meta: String,
    pub body: Vec<u8>,
}
enum Wait {
    Event(usize, usize),
    Network,
    Clock(u64),
    Yield,
    None,
}
struct State {
    limits: StoreLimits,
    wait: Wait,
    events: VecDeque<Vec<u8>>,
    request: Option<Request>,
    headers: String,
    body: Vec<u8>,
    response_code: Option<i32>,
    frame: Option<Frame>,
    error: Option<String>,
    epoch: u64,
    mono: u64,
    deadline: u64,
    line: String,
}
fn bad() -> Error {
    Error::new("Invalid browser buffer")
}
fn span(a: usize, n: usize, limit: usize) -> Result<core::ops::Range<usize>, Error> {
    let end = a.checked_add(n).filter(|v| *v <= limit).ok_or_else(bad)?;
    Ok(a..end)
}
fn arg(v: &[Val], i: usize) -> usize {
    v[i].i32().unwrap_or_default() as u32 as usize
}
fn pause() -> Error {
    Error::new("suspended browser")
}
fn take_event(state: &mut State, mem: &mut [u8], a: usize, n: usize) -> Result<i32, Error> {
    let Some(buf) = state.events.pop_front() else {
        state.wait = Wait::Event(a, n);
        return Err(pause());
    };
    if buf.len() > n {
        return Err(bad());
    }
    let r = span(a, buf.len(), mem.len())?;
    mem[r].copy_from_slice(&buf);
    Ok(buf.len() as i32)
}
fn browser(state: &mut State, mem: &mut [u8], name: &str, v: &[Val]) -> Result<i32, Error> {
    let bytes = |p: usize, n: usize, cap: usize| {
        if n <= cap {
            span(p, n, mem.len())
        } else {
            Err(bad())
        }
    };
    match name {
        "event" => take_event(state, mem, arg(v, 0), arg(v, 1)),
        "request" => {
            let m = bytes(arg(v, 0), arg(v, 1), 16384)?;
            let b = bytes(arg(v, 2), arg(v, 3), 1 << 20)?;
            state.request = Some(Request {
                meta: core::str::from_utf8(&mem[m]).map_err(|_| bad())?.into(),
                body: mem[b].to_vec(),
            });
            state.wait = Wait::Network;
            Err(pause())
        }
        "response" => {
            let src = if arg(v, 2) == 0 {
                state.headers.as_bytes()
            } else {
                &state.body
            };
            if src.len() > arg(v, 1) {
                return Ok(-1);
            }
            let r = span(arg(v, 0), src.len(), mem.len())?;
            mem[r].copy_from_slice(src);
            let n = src.len();
            if arg(v, 2) != 0 {
                state.body = Vec::new()
            }
            Ok(n as i32)
        }
        "present" => {
            let (width, height) = (arg(v, 1), arg(v, 2));
            if width == 0 || height == 0 || width > 4096 || height > 2160 {
                return Err(bad());
            }
            let r = bytes(arg(v, 0), width * height * 4, 4096 * 2160 * 4)?;
            let m = bytes(arg(v, 5), arg(v, 6), 16384)?;
            let mut meta = core::str::from_utf8(&mem[m])
                .map_err(|_| bad())?
                .splitn(4, '\n');
            let status = meta.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let url = meta.next().ok_or_else(bad)?.into();
            let ready = meta.next() == Some("ready");
            let title = meta.next().unwrap_or("").into();
            let pixels = mem[r]
                .chunks_exact(4)
                .map(|p| u32::from_be_bytes([0, p[0], p[1], p[2]]))
                .collect();
            state.frame = Some(Frame {
                ready,
                width,
                height,
                document_height: arg(v, 3),
                scroll: arg(v, 4),
                status,
                url,
                title,
                pixels,
            });
            Ok(0)
        }
        "notice" => {
            let r = bytes(arg(v, 0), arg(v, 1), 16384)?;
            state.error = Some(String::from_utf8_lossy(&mem[r]).into_owned());
            Ok(0)
        }
        _ => Err(Error::new("Unknown browser import")),
    }
}
pub struct Session<P: Platform> {
    store: Store<State>,
    memory: Memory,
    entry: Func,
    call: Option<ResumableCall>,
    _platform: core::marker::PhantomData<P>,
}
impl<P: Platform> Session<P> {
    pub fn new(lib: &Library, budget: usize) -> Result<Self, Error> {
        let mut linker = Linker::new(&lib.engine);
        let mut linked = BTreeSet::new();
        for imp in lib.module.imports() {
            if !linked.insert((imp.module(), imp.name())) {
                continue;
            }
            let ExternType::Func(ty) = imp.ty() else {
                return Err(Error::new("Unexpected browser import type"));
            };
            let is_wasi = imp.module() == "wasi_snapshot_preview1";
            if !is_wasi && imp.module() != "bos" {
                return Err(Error::new("Unexpected browser import namespace"));
            }
            let name = imp.name().to_string();
            linker.func_new(
                imp.module(),
                imp.name(),
                ty.clone(),
                move |mut c: Caller<State>, v, out| {
                    let memory = c
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .ok_or_else(bad)?;
                    let (m, state) = memory.data_and_store_mut(&mut c);
                    let value = if is_wasi {
                        wasi::call::<P>(state, m, &name, v)?
                    } else {
                        browser(state, m, &name, v)?
                    };
                    if let Some(result) = out.first_mut() {
                        *result = Val::I32(value)
                    }
                    Ok(())
                },
            )?;
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(budget)
            .instances(1)
            .memories(1)
            .build();
        let mut store = Store::new(
            &lib.engine,
            State {
                limits,
                wait: Wait::None,
                events: VecDeque::new(),
                request: None,
                headers: String::new(),
                body: Vec::new(),
                response_code: None,
                frame: None,
                error: None,
                epoch: P::epoch_ms(),
                mono: P::monotonic_ms(),
                deadline: 0,
                line: String::new(),
            },
        );
        store.limiter(|s| &mut s.limits);
        let instance = linker.instantiate_and_start(&mut store, &lib.module)?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(bad)?;
        let entry = instance.get_func(&store, "_start").ok_or_else(bad)?;
        Ok(Self {
            store,
            memory,
            entry,
            call: None,
            _platform: core::marker::PhantomData,
        })
    }
    pub fn event(&mut self, kind: u32, a: i32, b: i32, s: &str) {
        if s.len() > 8178 {
            return;
        }
        let q = &mut self.store.data_mut().events;
        if kind == 1 {
            q.clear()
        }
        if q.len() >= 128 {
            return;
        }
        let mut e = Vec::with_capacity(12 + s.len());
        e.extend(kind.to_le_bytes());
        e.extend(a.to_le_bytes());
        e.extend(b.to_le_bytes());
        e.extend(s.as_bytes());
        q.push_back(e);
    }
    pub fn take_request(&mut self) -> Option<Request> {
        self.store.data_mut().request.take()
    }
    pub fn respond(&mut self, result: Result<(String, Vec<u8>), String>) {
        let state = self.store.data_mut();
        let (meta, body, size) = match result {
            Ok((meta, body)) => {
                let size = body.len() as i32;
                (meta, body, size)
            }
            Err(e) => (e, Vec::new(), -1),
        };
        state.headers = meta;
        state.body = body;
        state.response_code = Some(size);
    }
    pub fn take_frame(&mut self) -> Option<Frame> {
        self.store.data_mut().frame.take()
    }
    pub fn take_error(&mut self) -> Option<String> {
        self.store.data_mut().error.take()
    }
    pub fn runnable(&self) -> bool {
        let s = self.store.data();
        match s.wait {
            Wait::Event(..) => !s.events.is_empty(),
            Wait::Network => s.response_code.is_some(),
            Wait::Clock(deadline) => P::monotonic_ms() >= deadline,
            Wait::Yield | Wait::None => true,
        }
    }
    pub fn step(&mut self, budget: u64) -> Result<(), Error> {
        if !self.runnable() {
            return Ok(());
        }
        self.store.data_mut().deadline = P::monotonic_ms().saturating_add(budget);
        let next = match self.call.take() {
            None => self.entry.call_resumable(&mut self.store, &[], &mut [])?,
            Some(ResumableCall::OutOfFuel(c)) => c.resume(&mut self.store, &mut [])?,
            Some(ResumableCall::HostTrap(c)) => {
                let (m, s) = self.memory.data_and_store_mut(&mut self.store);
                let val = match core::mem::replace(&mut s.wait, Wait::None) {
                    Wait::Event(a, n) => take_event(s, m, a, n)?,
                    Wait::Network => s.response_code.take().ok_or_else(bad)?,
                    Wait::Clock(_) | Wait::Yield => 0,
                    Wait::None => return Err(Error::new(c.host_error().to_string())),
                };
                c.resume(&mut self.store, &[Val::I32(val)], &mut [])?
            }
            Some(ResumableCall::Finished) => return Err(Error::new("Browser process exited")),
        };
        self.call = Some(next);
        Ok(())
    }
}
