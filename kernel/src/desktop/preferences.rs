use crate::fs::VFS;
use alloc::{format, string::String};
const DIR: &str = "/users/default/.config";
const PATH: &str = "/users/default/.config/desktop.cfg";
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}
impl Lang {
    pub fn t<'a>(self, en: &'a str, ru: &'a str) -> &'a str {
        if self == Self::Ru {
            ru
        } else {
            en
        }
    }
    pub fn code(self) -> &'static str {
        self.t("en", "ru")
    }
}
#[derive(Clone, Copy)]
pub struct Preferences {
    pub lang: Lang,
    pub dark: bool,
    pub accent: u8,
    pub wall: u8,
    pub animations: bool,
    pub speed: u8,
    pub shadows: bool,
    pub glass: bool,
    pub seconds: bool,
    pub mouse: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            lang: Lang::En,
            dark: false,
            accent: 0,
            wall: 0,
            animations: true,
            speed: 1,
            shadows: true,
            glass: true,
            seconds: false,
            mouse: 1,
        }
    }
}
impl Preferences {
    pub fn motion_ms(&self) -> u64 {
        if self.animations {
            [240, 180, 120][self.speed as usize]
        } else {
            0
        }
    }

    pub fn load() -> Self {
        let mut p = Self::default();
        if let Ok(data) = VFS.lock().read_file(PATH) {
            for l in String::from_utf8_lossy(&data).lines() {
                let Some((k, v)) = l.split_once('=') else {
                    continue;
                };
                let n = v.parse::<u8>().unwrap_or(0);
                match k {
                    "language" => p.lang = if v == "ru" { Lang::Ru } else { Lang::En },
                    "dark" => p.dark = n != 0,
                    "accent" => p.accent = n.min(5),
                    "wallpaper" => p.wall = n.min(4),
                    "animations" => p.animations = n != 0,
                    "speed" => p.speed = n.min(2),
                    "shadows" => p.shadows = n != 0,
                    "glass" => p.glass = n != 0,
                    "seconds" => p.seconds = n != 0,
                    "mouse" => p.mouse = n.min(3),
                    _ => {}
                }
            }
        }
        p
    }
    pub fn save(self) -> bool {
        let data=format!("language={}\ndark={}\naccent={}\nwallpaper={}\nanimations={}\nspeed={}\nshadows={}\nglass={}\nseconds={}\nmouse={}\n",self.lang.code(),self.dark as u8,self.accent,self.wall,self.animations as u8,self.speed,self.shadows as u8,self.glass as u8,self.seconds as u8,self.mouse);
        let mut v = VFS.lock();
        v.mkdir_p(DIR)
            .and_then(|_| v.write_file(PATH, data.as_bytes()))
            .and_then(|_| v.sync())
            .is_ok()
    }
    pub fn accent(self) -> u32 {
        [0x6054d2, 0x2970ca, 0x13805e, 0xa05a16, 0xb74170, 0x5b6377][self.accent as usize]
    }
    pub fn paper(self) -> u32 {
        if self.dark {
            0x252631
        } else {
            0xf9f8fb
        }
    }
    pub fn bar(self) -> u32 {
        if self.dark {
            0x31323e
        } else {
            0xeae9ef
        }
    }
    pub fn ink(self) -> u32 {
        if self.dark {
            0xf0eff6
        } else {
            0x22222e
        }
    }
    pub fn muted(self) -> u32 {
        if self.dark {
            0xb7b6c5
        } else {
            0x646373
        }
    }
    pub fn line(self) -> u32 {
        if self.dark {
            0x454552
        } else {
            0xd7d5e0
        }
    }
}
