//! ClaudeOS i386 executable and int 0x80 ABI, hosted in long-mode compatibility mode.
use super::*;
use alloc::string::String;
const BASE: u64 = 0x4000_0000;
const TOP: u64 = 0xbfff_e000;
const STACK: u64 = 64 * 1024;
pub(super) struct State {
    floor: u64,
    brk: u64,
    committed: u64,
    wake: Option<u64>,
    line: Vec<u8>,
}
fn word(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn long(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
pub(super) fn load(bytes: &[u8], args: &[&str]) -> Result<Program, &'static str> {
    if bytes.len() < 52
        || bytes[..7] != *b"\x7fELF\x01\x01\x01"
        || word(bytes, 16) != 2
        || word(bytes, 18) != 3
        || long(bytes, 20) != 1
        || word(bytes, 40) != 52
        || word(bytes, 42) != 32
    {
        return Err("expected a static i386 executable");
    }
    let count = word(bytes, 44) as usize;
    let off = long(bytes, 28) as usize;
    if !(1..=64).contains(&count) || off < 52 {
        return Err("invalid i386 program header");
    }
    let headers = bytes
        .get(off..off + count * 32)
        .ok_or("truncated i386 headers")?;
    let entry = long(bytes, 24) as u64;
    let mut p = Program::new()?;
    let low = p.frames.allocate_frame().ok_or("out of memory")?;
    unsafe {
        let root = &mut *(p.root.start_address().as_u64() as *mut PageTable);
        let old = root[0].addr().as_u64() as *const PageTable;
        let table = low.start_address().as_u64() as *mut PageTable;
        ptr::copy_nonoverlapping(old, table, 1);
        (*table)[1].set_unused();
        (*table)[2].set_unused();
        let flags = root[0].flags() | Flags::USER_ACCESSIBLE | Flags::WRITABLE;
        root[0].set_frame(low, flags);
    }
    let mut end = BASE;
    let mut valid = false;
    for h in headers.chunks_exact(32) {
        match long(h, 0) {
            2 | 3 | 7 => return Err("i386 dynamic linking is unavailable"),
            1 => {}
            _ => continue,
        }
        let at = long(h, 4) as usize;
        let addr = long(h, 8) as u64;
        let file = long(h, 16) as usize;
        let size = long(h, 20) as u64;
        let flags = long(h, 24);
        if size == 0 && file == 0 {
            continue;
        }
        if addr < BASE
            || addr + size > BASE + 128 * 1024 * 1024
            || file as u64 > size
            || flags & 3 == 3
            || flags & 4 == 0
            || addr % PAGE != at as u64 % PAGE
        {
            return Err("invalid i386 segment");
        }
        let data = bytes.get(at..at + file).ok_or("truncated i386 segment")?;
        p.load_range(&Segment {
            address: addr,
            size,
            bytes: data,
            writable: flags & 2 != 0,
            executable: flags & 1 != 0,
        })?;
        valid |= flags & 1 != 0 && (addr..addr + file as u64).contains(&entry);
        end = end.max(addr + size);
    }
    if !valid {
        return Err("i386 entry outside executable segment");
    }
    for va in (TOP - STACK..TOP).step_by(PAGE as usize) {
        p.page(va, Flags::WRITABLE | Flags::NO_EXECUTE)?;
    }
    let mut sp = TOP;
    let mut argv = Vec::new();
    if args.len() > 128 {
        return Err("too many arguments");
    }
    for arg in args {
        if arg.len() > 4095 {
            return Err("argument too long");
        }
        sp = sp
            .checked_sub(arg.len() as u64 + 1)
            .filter(|v| *v >= TOP - STACK + 2048)
            .ok_or("arguments exceed stack")?;
        p.write_user(sp, arg.as_bytes())
            .map_err(|_| "invalid startup stack")?;
        argv.push(sp as u32);
    }
    argv.push(0);
    sp = (sp & !15) - (argv.len() * 4) as u64;
    let vector = sp as u32;
    for (i, ptr) in argv.iter().enumerate() {
        p.write_user(sp + i as u64 * 4, &ptr.to_le_bytes())
            .map_err(|_| "invalid startup stack")?;
    }
    sp = (sp & !15) - 8;
    p.write_user(sp, &(args.len() as u32).to_le_bytes())
        .map_err(|_| "invalid argc")?;
    p.write_user(sp + 4, &vector.to_le_bytes())
        .map_err(|_| "invalid argv")?;
    let floor = end.next_multiple_of(PAGE);
    p.linux.cwd = crate::fs::VFS.lock().get_cwd();
    p.i386 = Some(State {
        floor,
        brk: floor,
        committed: floor,
        wake: None,
        line: Vec::new(),
    });
    p.context.rip = entry;
    p.context.cs = gdt::USER_CS32.0 as u64;
    p.context.rsp = sp;
    Ok(p)
}
pub(super) fn call(p: &mut Program, nr: u64, a: u64, b: u64, c: u64) -> i64 {
    let result: Result<i64, i64> = (|| match nr {
        1 | 2 if nr == 2 && a == 0 => {
            if c == 0 {
                return Ok(0);
            }
            let len = (c as usize).min(4096);
            p.validate_write(b, len)?;
            let state = p.i386.as_mut().unwrap();
            while state.line.len() < len && !state.line.ends_with(b"\n") {
                let ch = input_byte().ok_or(-11i64)?;
                match ch {
                    8 | 127 => {
                        state.line.pop();
                    }
                    b'\r' | b'\n' => state.line.push(b'\n'),
                    n => state.line.push(n),
                }
            }
            let bytes = core::mem::take(&mut state.line);
            let n = bytes.len();
            p.write_user(b, &bytes)?;
            Ok(n as i64)
        }
        1 | 2 => p.file_call(if nr == 1 { 1 } else { 0 }, a, b, c.min(4096)),
        3 => {
            let path = crate::fs::normalize_path(&p.linux.cwd, &p.path(a)?).map_err(|_| -1i64)?;
            let slot = (3..64).find(|&i| p.descriptors[i].is_none()).ok_or(-1i64)?;
            let fd = crate::fs::VFS.lock().open(&path, 0).map_err(|_| -1i64)?;
            p.descriptors[slot] = Some(Arc::new(Handle::File(fd)));
            Ok(slot as i64)
        }
        4 => p.file_call(3, a, 0, 0),
        5 => Ok(p.pid as i64),
        6 => {
            let now = crate::boottime::elapsed_ms();
            let wake = &mut p.i386.as_mut().unwrap().wake;
            let until = *wake.get_or_insert(now + a);
            if now < until {
                return Err(-11);
            }
            *wake = None;
            Ok(0)
        }
        7 => Ok(crate::boottime::elapsed_ms() as i64),
        8 => {
            let state = p.i386.as_ref().unwrap();
            let old = state.brk;
            let new = old
                .checked_add_signed(a as u32 as i32 as i64)
                .filter(|&v| v >= state.floor && v <= TOP - STACK - PAGE * 16)
                .ok_or(-1i64)?;
            while p.i386.as_ref().unwrap().committed < new {
                let va = p.i386.as_ref().unwrap().committed;
                p.page(va, Flags::WRITABLE | Flags::NO_EXECUTE)
                    .map_err(|_| -1i64)?;
                p.i386.as_mut().unwrap().committed += PAGE;
            }
            p.i386.as_mut().unwrap().brk = new;
            Ok(old as i64)
        }
        9 => Ok(0),
        10 => Ok(input_byte().map_or(-1, i64::from)),
        11 => {
            let path: String =
                crate::fs::normalize_path(&p.linux.cwd, &p.path(a)?).map_err(|_| -1i64)?;
            let st = crate::fs::VFS.lock().stat(&path).map_err(|_| -1i64)?;
            Ok(if st.is_dir() { -4 } else { st.size as i64 })
        }
        _ => Err(-1),
    })();
    result.unwrap_or_else(|v| v)
}
