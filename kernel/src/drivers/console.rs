//! Console over serial + GOP framebuffer text (8x16 font).
//!
//! `init()` latches the bootloader-provided framebuffer (identity-mapped
//! physical memory below 4GB) and clears it; `print()` writes to serial and,
//! when available, draws to the screen. 32bpp RGB/BGR(A) only — anything else
//! stays serial-only.
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, AtomicU32, AtomicU8, Ordering};
use spin::Mutex;

use bloader_shared::{FramebufferInfo, PixelFormat};

const FMT_NONE: u8 = 0;
const FMT_RGB: u8 = 1; // RGB byte order, little-endian word 0x00BBGGRR
const FMT_BGR: u8 = 2; // BGR byte order, little-endian word 0x00RRGGBB

const PALETTE: [u32; 16] = [
    0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa,
    0x555555, 0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
];

#[derive(Default)]
enum Escape {
    #[default]
    Text,
    Start,
    Csi { params: [u8; 8], index: usize, invalid: bool },
}

struct Fb {
    base: u64,
    width: u32,
    height: u32,
    stride: u32, // pixels per row
    fmt: u8,
    x: u32, // cursor cell
    y: u32,
    fg: u32,
    bg: u32,
    escape: Escape,
}

static FB: Mutex<Fb> = Mutex::new(Fb {
    base: 0,
    width: 0,
    height: 0,
    stride: 0,
    fmt: FMT_NONE,
    x: 0,
    y: 0,
    fg: 0xffffff,
    bg: 0,
    escape: Escape::Text,
});

// Re-exported for bSH diagnostics.
static FB_ON: AtomicU8 = AtomicU8::new(0);
static FB_BASE: AtomicU64 = AtomicU64::new(0);
static FB_WH: AtomicU32 = AtomicU32::new(0); // width<<16 | height

pub fn init(fb: Option<FramebufferInfo>) {
    let Some(f) = fb else {
        log::info!("console: no framebuffer, serial only");
        return;
    };
    if f.width < 8 || f.height < 16 || f.stride < f.width {
        log::info!("console: invalid framebuffer dimensions, serial only");
        return;
    }
    if f.bits_per_pixel != 32 {
        log::info!("console: {}bpp unsupported, serial only", f.bits_per_pixel);
        return;
    }
    let fmt = match f.pixel_format {
        PixelFormat::RGB | PixelFormat::RGBA => FMT_RGB,
        PixelFormat::BGR | PixelFormat::BGRA => FMT_BGR,
        PixelFormat::Bitmask => FMT_NONE,
    };
    if fmt == FMT_NONE {
        log::info!("console: pixel format unsupported, serial only");
        return;
    }
    // Identity-mapped low memory only (early page tables cover 0-4GB).
    let size = f.stride as u64 * f.height as u64 * 4;
    if f.base_addr == 0 || f.base_addr.saturating_add(size) > 0x1_0000_0000 {
        log::info!("console: fb outside identity map, serial only");
        return;
    }
    {
        let mut s = FB.lock();
        s.base = f.base_addr;
        s.width = f.width;
        s.height = f.height;
        s.stride = f.stride;
        s.fmt = fmt;
        s.x = 0;
        s.y = 0;
    }
    FB_ON.store(1, Ordering::SeqCst);
    FB_BASE.store(f.base_addr, Ordering::SeqCst);
    FB_WH.store((f.width << 16) | (f.height & 0xFFFF), Ordering::SeqCst);
    clear_fb();
    log::info!(
        "console: fb {}x{} stride {} at 0x{:x}",
        f.width,
        f.height,
        f.stride,
        f.base_addr
    );
}

pub fn is_fb_on() -> bool {
    FB_ON.load(Ordering::SeqCst) != 0
}

pub fn resolution() -> Option<(u32, u32)> {
    if !is_fb_on() {
        return None;
    }
    let dimensions = FB_WH.load(Ordering::SeqCst);
    Some((dimensions >> 16, dimensions & 0xFFFF))
}

/// Shell colors shared by the serial terminal and GOP renderer.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Color {
    Red = 91,
    Green = 92,
    Yellow = 93,
    Blue = 94,
    White = 97,
}

static FOREGROUND: AtomicU8 = AtomicU8::new(Color::White as u8);

fn foreground(code: u8) {
    print(match code {
        91 => "\x1b[91m",
        92 => "\x1b[92m",
        93 => "\x1b[93m",
        94 => "\x1b[94m",
        _ => "\x1b[97m",
    });
}

/// Restore the surrounding color when interactive commands or errors return.
pub struct ColorGuard(u8);

impl ColorGuard {
    pub fn new(color: Color) -> Self {
        let previous = FOREGROUND.swap(color as u8, Ordering::SeqCst);
        foreground(color as u8);
        Self(previous)
    }
}

impl Drop for ColorGuard {
    fn drop(&mut self) {
        FOREGROUND.store(self.0, Ordering::SeqCst);
        foreground(self.0);
    }
}

pub fn print_colored(text: &str, color: Color) {
    let _color = ColorGuard::new(color);
    print(text);
}

pub fn print(s: &str) {
    crate::arch::x86_64::serial::write_str(s);
    if !is_fb_on() {
        return;
    }
    let mut fb = FB.lock();
    for ch in s.chars() {
        putc(&mut fb, ch);
    }
}

/// A shared viewport that fits an ordinary 80x24 serial terminal.
pub fn editor_dimensions() -> (usize, usize) {
    match resolution() {
        Some((width, height)) => ((width as usize / 8).min(80), (height as usize / 16).min(24)),
        None => (80, 24),
    }
}

pub fn begin_editor() {
    crate::arch::x86_64::serial::write_str("\x1b[?1049h\x1b[?25l");
    clear();
}

pub fn end_editor() {
    if is_fb_on() {
        clear_fb();
        let mut fb = FB.lock();
        fb.x = 0;
        fb.y = 0;
    }
    crate::arch::x86_64::serial::write_str("\x1b[0m\x1b[?25h\x1b[?1049l");
}

pub fn editor_cursor(row: usize, column: usize) {
    crate::arch::x86_64::serial::write_str(&alloc::format!("\x1b[{};{}H", row + 1, column + 1));
    if is_fb_on() {
        let mut fb = FB.lock();
        fb.y = (row as u32).min(fb.height / 16 - 1);
        fb.x = (column as u32).min(fb.width / 8 - 1);
    }
}

pub fn editor_row(row: usize, text: &str) {
    editor_cursor(row, 0);
    crate::arch::x86_64::serial::write_str("\x1b[K");
    if is_fb_on() {
        let fb = FB.lock();
        let start = row * 16 * fb.stride as usize;
        if row < (fb.height / 16) as usize {
            fill(unsafe { (fb.base as *mut u32).add(start) }, 16 * fb.stride as usize, fb.pixel(0));
        }
    }
    print(text);
}

pub fn editor_caret(visible: bool) {
    crate::arch::x86_64::serial::write_str(if visible { "\x1b[?25h" } else { "\x1b[?25l" });
    toggle_input_cursor();
}

/// Toggle a cursor underline while the line editor is waiting for input.
pub fn toggle_input_cursor() {
    if !is_fb_on() {
        return;
    }
    let fb = FB.lock();
    if fb.x >= fb.width / 8 {
        return;
    }
    let start = (fb.y * 16 + 15) * fb.stride + fb.x * 8;
    for col in 0..8 {
        unsafe {
            let pixel = (fb.base as *mut u32).add((start + col) as usize);
            pixel.write_volatile(pixel.read_volatile() ^ 0x00FF_FFFF);
        }
    }
}

/// Clear both outputs: ANSI for serial terms, fill for the framebuffer.
pub fn clear() {
    crate::arch::x86_64::serial::write_str("\x1b[2J\x1b[H");
    if is_fb_on() {
        clear_fb();
        let mut fb = FB.lock();
        fb.x = 0;
        fb.y = 0;
    }
}

fn clear_fb() {
    let fb = FB.lock();
    fill(fb.base as *mut u32, fb.stride as usize * fb.height as usize, fb.pixel(fb.bg));
}

impl Fb {
    fn pixel(&self, rgb: u32) -> u32 {
        if self.fmt == FMT_RGB {
            (rgb & 0x00ff00) | ((rgb & 0xff0000) >> 16) | ((rgb & 0x0000ff) << 16)
        } else {
            rgb
        }
    }

    fn sgr(&mut self, params: &[u8]) {
        let mut params = params.iter().copied();
        while let Some(code) = params.next() {
            match code {
                0 => {
                    self.fg = 0xffffff;
                    self.bg = 0;
                }
                30..=37 => self.fg = PALETTE[(code - 30) as usize],
                39 => self.fg = 0xffffff,
                40..=47 => self.bg = PALETTE[(code - 40) as usize],
                49 => self.bg = 0,
                90..=97 => self.fg = PALETTE[(code - 90 + 8) as usize],
                100..=107 => self.bg = PALETTE[(code - 100 + 8) as usize],
                38 | 48 => match params.next() {
                    Some(2) => { params.by_ref().take(3).for_each(drop); }
                    Some(5) => { params.next(); }
                    _ => break,
                },
                _ => {}
            }
        }
    }
}

fn putc(fb: &mut Fb, ch: char) {
    if ch == '\x1b' {
        fb.escape = Escape::Start;
        return;
    }
    match core::mem::take(&mut fb.escape) {
        Escape::Start if ch == '[' => {
            fb.escape = Escape::Csi { params: [0; 8], index: 0, invalid: false };
            return;
        }
        Escape::Csi { mut params, mut index, mut invalid } => match ch {
            '@'..='~' => {
                if ch == 'm' && !invalid {
                    fb.sgr(&params[..=index]);
                }
                return;
            }
            ' '..='?' => {
                match ch {
                    '0'..='9' => {
                        params[index] = params[index].saturating_mul(10).saturating_add(ch as u8 - b'0');
                    }
                    ';' if index + 1 < params.len() => index += 1,
                    _ => invalid = true,
                }
                fb.escape = Escape::Csi { params, index, invalid };
                return;
            }
            _ => {}
        },
        _ => {}
    }
    let cols = fb.width / 8;
    let rows = fb.height / 16;
    match ch {
        '\n' => {
            fb.x = 0;
            fb.y += 1;
        }
        '\r' => fb.x = 0,
        '\t' => fb.x = ((fb.x + 8) & !7).min(cols),
        '\x08' => {
            if fb.x > 0 {
                fb.x -= 1;
            } else if fb.y > 0 {
                fb.y -= 1;
                fb.x = cols - 1;
            }
        }
        ch if ch.is_control() => {}
        _ => {
            let g = if (' '..='~').contains(&ch) {
                ch as u8
            } else {
                b'?'
            };
            if fb.x >= cols {
                fb.x = 0;
                fb.y += 1;
            }
            if fb.y >= rows {
                scroll(fb);
                fb.y = rows - 1;
            }
            draw_glyph(fb, g);
            fb.x += 1;
        }
    }
    if fb.y >= rows {
        scroll(fb);
        fb.y = rows - 1;
    }
}

fn draw_glyph(fb: &Fb, g: u8) {
    let glyph = &super::font8x16::FONT[g as usize];
    let base = fb.base as *mut u32;
    let fg = fb.pixel(fb.fg);
    let bg = fb.pixel(fb.bg);
    for row in 0..16 {
        let bits = glyph[row];
        for col in 0..8 {
            let px = fb.x * 8 + col;
            let py = fb.y * 16 + row as u32;
            let color = if bits & (0x80 >> col) == 0 { bg } else { fg };
            unsafe {
                core::ptr::write_volatile(base.add((py * fb.stride + px) as usize), color);
            }
        }
    }
}

fn fill(base: *mut u32, words: usize, color: u32) {
    for index in 0..words {
        unsafe { base.add(index).write_volatile(color); }
    }
}

fn scroll(fb: &Fb) {
    let row_words = fb.stride as usize * 16;
    let visible_words = row_words * (fb.height / 16) as usize;
    let base = fb.base as *mut u32;
    unsafe {
        core::ptr::copy(base.add(row_words), base, visible_words - row_words);
        fill(base.add(visible_words - row_words), row_words, fb.pixel(fb.bg));
    }
}

pub struct Console;
impl Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        print(s);
        Ok(())
    }
}
