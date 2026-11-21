use super::*;
fn put(m: &mut [u8], at: usize, data: &[u8]) -> Result<(), Error> {
    let r = span(at, data.len(), m.len())?;
    m[r].copy_from_slice(data);
    Ok(())
}
fn u32at(m: &[u8], at: usize) -> Result<usize, Error> {
    Ok(u32::from_le_bytes(m[span(at, 4, m.len())?].try_into().unwrap()) as usize)
}
fn u64at(m: &[u8], at: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        m[span(at, 8, m.len())?].try_into().unwrap(),
    ))
}
pub(super) fn call<P: Platform>(
    s: &mut State,
    m: &mut [u8],
    name: &str,
    v: &[Val],
) -> Result<i32, Error> {
    let a = |i| arg(v, i);
    match name {
        "args_sizes_get" => {
            put(m, a(0), &1u32.to_le_bytes())?;
            put(m, a(1), &8u32.to_le_bytes())?;
        }
        "args_get" => {
            put(m, a(0), &(a(1) as u32).to_le_bytes())?;
            put(m, a(1), b"browser\0")?;
        }
        "environ_sizes_get" => {
            put(m, a(0), &0u32.to_le_bytes())?;
            put(m, a(1), &0u32.to_le_bytes())?;
        }
        "environ_get" => {}
        "sched_yield" => {
            if P::monotonic_ms() >= s.deadline {
                s.wait = Wait::Yield;
                return Err(pause());
            }
        }
        "clock_time_get" => {
            let now = P::monotonic_ms();
            let ns = if a(0) == 0 {
                s.epoch.saturating_add(now.saturating_sub(s.mono))
            } else {
                now
            } * 1_000_000;
            put(m, a(2), &ns.to_le_bytes())?;
        }
        "random_get" => {
            let r = span(a(0), a(1), m.len())?;
            if !P::random(&mut m[r]) {
                return Ok(29);
            }
        }
        "fd_prestat_get" | "fd_prestat_dir_name" => return Ok(8),
        "fd_fdstat_get" if a(0) < 3 => {
            let mut stat = [0; 24];
            stat[0] = 2;
            put(m, a(1), &stat)?;
        }
        "fd_fdstat_set_flags" if a(0) < 3 => {}
        "fd_write" if a(0) == 1 || a(0) == 2 => {
            let mut total = 0u32;
            for i in 0..a(2).min(1024) {
                let pos = a(1) + i * 8;
                let b = u32at(m, pos)?;
                let n = u32at(m, pos + 4)?;
                let r = span(b, n, m.len())?;
                total = total.saturating_add(n as u32);
                for ch in String::from_utf8_lossy(&m[r]).chars() {
                    if ch == '\n' || s.line.len() > 2048 {
                        P::log(&s.line);
                        s.line.clear()
                    } else {
                        s.line.push(ch);
                    }
                }
            }
            put(m, a(3), &total.to_le_bytes())?;
        }
        "poll_oneoff" => {
            if a(2) == 0 || a(2) > 128 {
                return Ok(28);
            }
            let mut soon = u64::MAX;
            let mut index = 0;
            for i in 0..a(2) {
                let pos = a(0) + 48 * i;
                let r = span(pos, 48, m.len())?;
                let sub = &m[r];
                if sub[8] != 0 {
                    continue;
                }
                let nanos = u64at(sub, 24)?;
                let duration = nanos.div_ceil(1_000_000);
                let now = P::monotonic_ms();
                let deadline = if sub[40] & 1 != 0 {
                    if u32at(sub, 16)? == 0 {
                        now.saturating_add(
                            duration.saturating_sub(s.epoch + now.saturating_sub(s.mono)),
                        )
                    } else {
                        duration
                    }
                } else {
                    now.saturating_add(duration)
                };
                if deadline < soon {
                    soon = deadline;
                    index = i
                }
            }
            if soon == u64::MAX {
                return Ok(58);
            }
            let data = u64at(m, a(0) + index * 48)?;
            let mut out = [0; 32];
            out[..8].copy_from_slice(&data.to_le_bytes());
            put(m, a(1), &out)?;
            put(m, a(3), &1u32.to_le_bytes())?;
            if soon > P::monotonic_ms() {
                s.wait = Wait::Clock(soon);
                return Err(pause());
            }
        }
        "proc_exit" => {
            return Err(Error::new(alloc::format!(
                "Browser exited with code {}",
                a(0)
            )))
        }
        name if name.starts_with("path_") => return Ok(76),
        name if name.starts_with("fd_") => return Ok(8),
        _ => return Ok(52),
    }
    Ok(0)
}
