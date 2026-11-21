//! PS/2 relative mouse and IntelliMouse wheel packets on the shared i8042.
use super::keyboard::read_controller;
use x86_64::instructions::port::Port;

fn send(port: u16, value: u8) -> bool {
    for _ in 0..10000 {
        let status = unsafe { Port::<u8>::new(0x64).read() };
        if status == 0xff {
            return false;
        }
        if status & 2 == 0 {
            unsafe {
                Port::<u8>::new(port).write(value);
            }
            return true;
        }
    }
    false
}

fn command(value: u8) -> bool {
    if !send(0x64, 0xd4) || !send(0x60, value) {
        return false;
    }
    (0..20000).any(|_| matches!(read_controller(), Some((true, 0xfa))))
}

pub fn init() {
    let _ = send(0x64, 0xa8);
}
pub fn enable() -> Option<Packets> {
    init();
    if !command(0xf6) {
        return None;
    }
    let wheel = [200, 100, 80]
        .iter()
        .all(|rate| command(0xf3) && command(*rate))
        && command(0xf2)
        && (0..20000)
            .find_map(|_| match read_controller() {
                Some((true, id)) => Some(id),
                _ => None,
            })
            .is_some_and(|id| matches!(id, 3 | 4));
    command(0xf4).then_some(Packets {
        wheel,
        ..Packets::default()
    })
}
pub fn disable() {
    let _ = command(0xf5);
}

#[derive(Default)]
pub struct Packets {
    bytes: [u8; 4],
    wheel: bool,
    count: usize,
}

impl Packets {
    pub fn push(&mut self, byte: u8) -> Option<(i32, i32, u8, i32)> {
        if self.count == 0 && byte & 8 == 0 {
            return None;
        }
        self.bytes[self.count] = byte;
        self.count += 1;
        if self.count < if self.wheel { 4 } else { 3 } {
            return None;
        }
        self.count = 0;
        let [flags, x, y, z] = self.bytes;
        let wheel = if self.wheel {
            i32::from(((z << 4) as i8) >> 4)
        } else {
            0
        };
        let delta = |value: u8, sign, overflow| {
            if flags & overflow != 0 {
                0
            } else {
                i32::from(value) - if flags & sign != 0 { 256 } else { 0 }
            }
        };
        Some((
            delta(x, 0x10, 0x40),
            -delta(y, 0x20, 0x80),
            flags & 7,
            wheel,
        ))
    }
}
