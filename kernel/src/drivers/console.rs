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
const FMT_RGB: u8 = 1; // 0x00RRGGBB
const FMT_BGR: u8 = 2; // 0x00BBGGRR

struct Fb {
    base: u64,
    width: u32,
    height: u32,
    stride: u32, // pixels per row
    fmt: u8,
    x: u32, // cursor cell
    y: u32,
}

static FB: Mutex<Fb> = Mutex::new(Fb {
    base: 0,
    width: 0,
    height: 0,
    stride: 0,
    fmt: FMT_NONE,
    x: 0,
    y: 0,
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

pub fn print(s: &str) {
    crate::arch::x86_64::serial::write_str(s);
    if !is_fb_on() {
        return;
    }
    let mut fb = FB.lock();
    for b in s.bytes() {
        putc(&mut fb, b);
    }
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
    let (base, stride, height) = {
        let s = FB.lock();
        (s.base, s.stride, s.height)
    };
    unsafe {
        core::ptr::write_bytes(base as *mut u32, 0, (stride as usize) * (height as usize));
    }
}

fn putc(fb: &mut Fb, b: u8) {
    let cols = fb.width / 8;
    let rows = fb.height / 16;
    match b {
        b'\n' => {
            fb.x = 0;
            fb.y += 1;
        }
        b'\r' => fb.x = 0,
        b'\t' => fb.x = (fb.x + 8) & !7,
        0x08 => {
            if fb.x > 0 {
                fb.x -= 1;
            } else if fb.y > 0 {
                fb.y -= 1;
                fb.x = cols - 1;
            }
        }
        _ => {
            let g = if (0x20..0x7f).contains(&b) {
                b
            } else {
                b'?'
            };
            if fb.x >= cols {
                fb.x = 0;
                fb.y += 1;
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
    for row in 0..16 {
        let bits = glyph[row];
        for col in 0..8 {
            let px = fb.x * 8 + col;
            let py = fb.y * 16 + row as u32;
            let color = if bits & (0x80 >> col) == 0 { 0 } else { 0x00FF_FFFF };
            unsafe {
                core::ptr::write_volatile(base.add((py * fb.stride + px) as usize), color);
            }
        }
    }
}

fn scroll(fb: &mut Fb) {
    let row_words = fb.stride as usize;
    let row_bytes = row_words * 4;
    let base = fb.base as *mut u8;
    unsafe {
        // Move rows 1..N up by one text row (16 px).
        core::ptr::copy(
            base.add(row_bytes * 16),
            base,
            row_bytes * (fb.height as usize - 16),
        );
        // Clear the last text row.
        core::ptr::write_bytes(
            base.add(row_bytes * (fb.height as usize - 16)),
            0,
            row_bytes * 16,
        );
    }
}

pub struct Console;
impl Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        print(s);
        Ok(())
    }
}
