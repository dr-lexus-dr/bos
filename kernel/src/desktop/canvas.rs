//! Software compositor. The framebuffer is touched only after a complete frame.
use alloc::{vec, vec::Vec};
use bloader_shared::{FramebufferInfo, PixelFormat};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
pub const fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}
impl Rect {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn inset(self, n: i32) -> Self {
        rect(self.x + n, self.y + n, self.w - 2 * n, self.h - 2 * n)
    }
    pub(super) fn intersection(self, b: Self) -> Self {
        let x = self.x.max(b.x);
        let y = self.y.max(b.y);
        rect(
            x,
            y,
            (self.x + self.w).min(b.x + b.w) - x,
            (self.y + self.h).min(b.y + b.h) - y,
        )
    }
}
pub fn mix(a: u32, b: u32, t: u32) -> u32 {
    let s = 256 - t;
    ((((a & 0xff00ff) * s + (b & 0xff00ff) * t) >> 8) & 0xff00ff)
        | ((((a & 0xff00) * s + (b & 0xff00) * t) >> 8) & 0xff00)
}
pub struct Picture {
    pub w: usize,
    pub h: usize,
    pixels: Vec<u8>,
}
impl Picture {
    pub fn jpeg(bytes: &[u8]) -> Option<Self> {
        let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
        decoder.decode_headers().ok()?;
        let (w, h) = decoder.dimensions()?;
        if w == 0 || h == 0 || w.checked_mul(h)? > 1_200_000 {
            return None;
        }
        let pixels = decoder.decode().ok()?;
        (pixels.len() == w * h * 3).then_some(Self { w, h, pixels })
    }
    fn at(&self, x: usize, y: usize) -> u32 {
        let i = (y * self.w + x) * 3;
        (self.pixels[i] as u32) << 16 | (self.pixels[i + 1] as u32) << 8 | self.pixels[i + 2] as u32
    }
    fn sample(&self, x: usize, y: usize) -> u32 {
        let l = (x >> 8).min(self.w - 1);
        let t = (y >> 8).min(self.h - 1);
        let r = (l + 1).min(self.w - 1);
        let b = (t + 1).min(self.h - 1);
        mix(
            mix(self.at(l, t), self.at(r, t), (x & 255) as u32),
            mix(self.at(l, b), self.at(r, b), (x & 255) as u32),
            (y & 255) as u32,
        )
    }
}
pub struct Canvas {
    fb: FramebufferInfo,
    pub w: i32,
    pub h: i32,
    buf: Vec<u32>,
    clip: Rect,
    wall: Vec<u32>,
    overview: Vec<u32>,
}
impl Canvas {
    pub fn new(fb: FramebufferInfo) -> Self {
        let (w, h) = (fb.width as i32, fb.height as i32);
        let len = (w * h) as usize;
        Self {
            fb,
            w,
            h,
            buf: vec![0; len],
            clip: rect(0, 0, w, h),
            wall: vec![0; len],
            overview: vec![0; len],
        }
    }
    pub fn set_wall(&mut self, photo: &Picture, style: u8) {
        let stride = self.w as usize;
        let sx = photo.w * 256 / stride;
        let sy = photo.h * 256 / self.h as usize;
        let scale = sx.min(sy).max(1);
        let ox = (photo.w * 256 - stride * scale) / 2;
        let oy = (photo.h * 256 - self.h as usize * scale) / 2;
        for y in 0..self.h as usize {
            for x in 0..stride {
                let raw = photo.sample(ox + x * scale, oy + y * scale);
                let c = match style {
                    1 => mix(raw, 0x49336c, 85),
                    2 => mix(raw, 0x125460, 90),
                    3 => mix(
                        0x233753,
                        0x929ae4,
                        ((x + y) * 245 / (stride + self.h as usize)) as u32,
                    ),
                    4 => mix(0x783e48, 0xefad80, (y * 255 / self.h as usize) as u32),
                    _ => raw,
                };
                self.wall[y * stride + x] = c;
            }
        }
        // Small separable box filter, expanded once. Overview redraws need no blur pass.
        let bw = (stride + 15) / 16;
        let bh = (self.h as usize + 15) / 16;
        let mut small = vec![0; bw * bh];
        for y in 0..bh {
            for x in 0..bw {
                let (mut r, mut g, mut b, mut n) = (0, 0, 0, 0);
                for j in (y * 16..((y + 1) * 16).min(self.h as usize)).step_by(2) {
                    for i in (x * 16..((x + 1) * 16).min(stride)).step_by(2) {
                        let v = self.wall[j * stride + i];
                        r += v >> 16;
                        g += (v >> 8) & 255;
                        b += v & 255;
                        n += 1;
                    }
                }
                small[y * bw + x] = mix((r / n) << 16 | (g / n) << 8 | b / n, 0x151728, 145);
            }
        }
        for y in 0..self.h as usize {
            let ty = (y / 16).min(bh - 1);
            let by = (ty + 1).min(bh - 1);
            for x in 0..stride {
                let lx = x / 16;
                let rx = (lx + 1).min(bw - 1);
                let fx = (x % 16 * 16) as u32;
                self.overview[y * stride + x] = mix(
                    mix(small[ty * bw + lx], small[ty * bw + rx], fx),
                    mix(small[by * bw + lx], small[by * bw + rx], fx),
                    (y % 16 * 16) as u32,
                );
            }
        }
    }
    pub fn snapshot(&self, r: Rect) -> Option<Vec<u32>> {
        if r.x < 0 || r.y < 0 || r.w <= 0 || r.h <= 0 || r.x + r.w > self.w || r.y + r.h > self.h {
            return None;
        }
        let mut pixels = Vec::new();
        pixels.try_reserve_exact((r.w * r.h) as usize).ok()?;
        for y in r.y..r.y + r.h {
            let i = (y * self.w + r.x) as usize;
            pixels.extend_from_slice(&self.buf[i..i + r.w as usize]);
        }
        Some(pixels)
    }
    pub fn backdrop(&self) -> Vec<u32> {
        self.buf.clone()
    }
    pub fn region_from(&mut self, screen: &[u32], area: Rect) {
        let r = area.intersection(rect(0, 0, self.w, self.h));
        if r.w <= 0 || r.h <= 0 {
            return;
        }
        for y in r.y..r.y + r.h {
            let i = (y * self.w + r.x) as usize;
            let end = i + r.w as usize;
            self.buf[i..end].copy_from_slice(&screen[i..end]);
        }
    }
    pub fn surface(&mut self, sprite: &[u32], source: Rect, target: Rect, opacity: u32) {
        if target.w <= 0 || target.h <= 0 {
            return;
        }
        let v = target.intersection(self.clip);
        if v.w <= 0 || v.h <= 0 {
            return;
        }
        let step = ((source.w as u64) << 16) / target.w as u64;
        let radius = 9.min(target.h / 2).min(target.w / 2);
        for y in v.y..v.y + v.h {
            let row = y - target.y;
            let dy = (radius - 1 - row.min(target.h - row - 1)).max(0);
            let cut = if dy == 0 {
                0
            } else {
                radius - (radius * radius - dy * dy).isqrt()
            };
            let x0 = (target.x + cut).max(v.x);
            let x1 = (target.x + target.w - cut).min(v.x + v.w);
            if x1 <= x0 {
                continue;
            }
            let i = (y * self.w + x0) as usize;
            let j = (row * source.h / target.h * source.w) as usize;
            let dest = &mut self.buf[i..i + (x1 - x0) as usize];
            if source.w == target.w && opacity == 256 {
                let lo = j + (x0 - target.x) as usize;
                let end = lo + dest.len();
                dest.copy_from_slice(&sprite[lo..end]);
            } else {
                let mut sx = (x0 - target.x) as u64 * step;
                for px in dest {
                    let color = sprite[j + (sx >> 16) as usize];
                    *px = if opacity == 256 {
                        color
                    } else {
                        mix(*px, color, opacity)
                    };
                    sx += step;
                }
            }
        }
    }
    pub fn begin(&mut self, overview: bool) {
        self.clip = rect(0, 0, self.w, self.h);
        self.buf
            .copy_from_slice(if overview { &self.overview } else { &self.wall });
    }
    pub fn clip(&mut self, r: Rect) -> Rect {
        let prev = self.clip;
        self.clip = prev.intersection(r);
        prev
    }
    pub fn restore(&mut self, r: Rect) {
        self.clip = r;
    }
    pub fn point(&mut self, x: i32, y: i32, c: u32) {
        if self.clip.contains(x, y) {
            self.buf[(y * self.w + x) as usize] = c;
        }
    }
    fn alpha(&mut self, x: i32, y: i32, c: u32, a: u32) {
        if self.clip.contains(x, y) {
            let p = &mut self.buf[(y * self.w + x) as usize];
            *p = mix(*p, c, a);
        }
    }
    pub fn fill(&mut self, r: Rect, c: u32) {
        let r = r.intersection(self.clip);
        if r.w <= 0 {
            return;
        }
        for y in r.y..r.y + r.h {
            let i = (y * self.w + r.x) as usize;
            self.buf[i..i + r.w as usize].fill(c);
        }
    }
    pub fn panel(&mut self, r: Rect, radius: i32, c: u32, opacity: u32) {
        if r.w <= 0 || r.h <= 0 {
            return;
        }
        let rad = radius.min(r.w / 2).min(r.h / 2).max(0);
        let clipped = r.intersection(self.clip);
        for y in clipped.y..clipped.y + clipped.h {
            let dy = (rad - (y - r.y).min(r.y + r.h - 1 - y) - 1).max(0);
            let inset = if dy == 0 {
                0
            } else {
                rad - (rad * rad - dy * dy).isqrt()
            };
            let left = (r.x + inset).max(clipped.x);
            let right = (r.x + r.w - inset).min(clipped.x + clipped.w);
            if left >= right {
                continue;
            }
            let i = (y * self.w + left) as usize;
            let line = &mut self.buf[i..i + (right - left) as usize];
            if opacity >= 256 {
                line.fill(c)
            } else {
                for p in line {
                    *p = mix(*p, c, opacity);
                }
            }
        }
    }
    pub fn shadow(&mut self, r: Rect) {
        let inner = r.inset(9);
        for z in [9, 6, 3] {
            let outer = rect(r.x - z, r.y + 3 - z / 2, r.w + 2 * z, r.h + z);
            for band in [
                rect(outer.x, outer.y, outer.w, inner.y - outer.y),
                rect(outer.x, inner.y, inner.x - outer.x, inner.h),
                rect(
                    inner.x + inner.w,
                    inner.y,
                    outer.x + outer.w - inner.x - inner.w,
                    inner.h,
                ),
                rect(
                    outer.x,
                    inner.y + inner.h,
                    outer.w,
                    outer.y + outer.h - inner.y - inner.h,
                ),
            ] {
                let old = self.clip(band);
                self.panel(outer, 12 + z, 0x131320, 18);
                self.restore(old);
            }
        }
    }
    pub fn round(&mut self, r: Rect, radius: i32, c: u32) {
        self.panel(r, radius, c, 256);
    }
    pub fn line(&mut self, mut x: i32, mut y: i32, tx: i32, ty: i32, c: u32, thick: i32) {
        let dx = (tx - x).abs();
        let sx = (tx - x).signum();
        let dy = -(ty - y).abs();
        let sy = (ty - y).signum();
        let mut e = dx + dy;
        loop {
            self.fill(rect(x, y, thick, thick), c);
            if x == tx && y == ty {
                break;
            }
            let t = 2 * e;
            if t >= dy {
                e += dy;
                x += sx;
            }
            if t <= dx {
                e += dx;
                y += sy;
            }
        }
    }
    /// Scale an antialiased indexed icon into the current clip.
    pub fn indexed(&mut self, r: Rect, palette: &[u8], pixels: &[u8], size: usize) {
        if r.w < 1 || r.h < 1 {
            return;
        }
        let clipped = r.intersection(self.clip);
        for y in clipped.y..clipped.y + clipped.h {
            let row = ((y - r.y) as usize * size / r.h as usize) * size;
            for x in clipped.x..clipped.x + clipped.w {
                let p = pixels[row + (x - r.x) as usize * size / r.w as usize] as usize * 4;
                let alpha = palette[p + 3];
                if alpha == 0 {
                    continue;
                }
                let color = (palette[p] as u32) << 16
                    | (palette[p + 1] as u32) << 8
                    | palette[p + 2] as u32;
                let i = (y * self.w + x) as usize;
                self.buf[i] = if alpha == 255 {
                    color
                } else {
                    mix(self.buf[i], color, alpha as u32)
                };
            }
        }
    }
    pub fn picture(&mut self, r: Rect, p: &Picture) {
        if r.w <= 0 || r.h <= 0 {
            return;
        }
        let c = r.intersection(self.clip);
        for y in c.y..c.y + c.h {
            for x in c.x..c.x + c.w {
                let v = p.sample(
                    ((x - r.x) as usize * p.w * 256) / r.w as usize,
                    ((y - r.y) as usize * p.h * 256) / r.h as usize,
                );
                self.point(x, y, v);
            }
        }
    }
    pub fn label(&mut self, x: i32, y: i32, s: &str, c: u32, font: usize, max: i32) {
        let mut pos = x;
        for ch in s.chars() {
            let (off, w, h, l, t, advance) = glyph(ch, font);
            if pos + advance > x + max {
                break;
            }
            for row in 0..h {
                for col in 0..w {
                    let n = (row * w + col) as usize;
                    let byte = FONT[off + n / 2];
                    let a = if n % 2 == 0 { byte >> 4 } else { byte & 15 };
                    if a != 0 {
                        self.alpha(
                            pos + l + col,
                            y + SIZES[font] + t + row,
                            c,
                            (a as u32 * 256) / 15,
                        );
                    }
                }
            }
            pos += advance;
        }
    }
    pub fn centered(&mut self, r: Rect, s: &str, c: u32, font: usize) {
        let width = width(s, font).min(r.w);
        self.label(
            r.x + (r.w - width) / 2,
            r.y + (r.h - SIZES[font] - 2) / 2,
            s,
            c,
            font,
            r.w,
        );
    }
    pub fn mono(&mut self, x: i32, y: i32, ch: char, color: u32) {
        const FONT: &[u8] = include_bytes!("font.bin");
        let index = match ch {
            ' '..='~' => ch as usize,
            'Ё' => 128,
            'ё' => 129,
            'А'..='я' => 130 + ch as usize - 0x410,
            _ => b'?' as usize,
        };
        for (dy, bits) in FONT[index * 16..index * 16 + 16].iter().enumerate() {
            for dx in 0..8 {
                if bits & (0x80 >> dx) != 0 {
                    self.point(x + dx, y + dy as i32, color)
                }
            }
        }
    }
    pub fn pixels(&mut self, r: Rect, data: &[u32], width: usize, height: usize) {
        let v =
            rect(r.x, r.y, r.w.min(width as i32), r.h.min(height as i32)).intersection(self.clip);
        if v.w <= 0 || v.h <= 0 || width.checked_mul(height) != Some(data.len()) {
            return;
        }
        for y in v.y..v.y + v.h {
            let i = (y * self.w + v.x) as usize;
            let j = (y - r.y) as usize * width + (v.x - r.x) as usize;
            self.buf[i..i + v.w as usize].copy_from_slice(&data[j..j + v.w as usize]);
        }
    }
    pub fn present(&self) {
        self.present_area(rect(0, 0, self.w, self.h));
    }
    pub fn present_area(&self, r: Rect) {
        let r = r.intersection(rect(0, 0, self.w, self.h));
        if r.w <= 0 {
            return;
        }
        for y in r.y..r.y + r.h {
            let i = (y * self.w + r.x) as usize;
            let dest = unsafe {
                (self.fb.base_addr as *mut u32)
                    .add(y as usize * self.fb.stride as usize + r.x as usize)
            };
            if self.fb.pixel_format == PixelFormat::BGR {
                unsafe {
                    core::ptr::copy_nonoverlapping(self.buf.as_ptr().add(i), dest, r.w as usize);
                }
            } else {
                for (dx, p) in self.buf[i..i + r.w as usize].iter().enumerate() {
                    unsafe {
                        dest.add(dx)
                            .write_volatile((p & 0xff00) | (p >> 16) | ((p & 255) << 16));
                    }
                }
            }
        }
    }
    pub fn cursor(&self, x: i32, y: i32) {
        const CUR: [&[u8]; 17] = [
            b"#",
            b"##",
            b"#.#",
            b"#..#",
            b"#...#",
            b"#....#",
            b"#.....#",
            b"#......#",
            b"#.......#",
            b"#........#",
            b"#.....#####",
            b"#..#..#",
            b"#.# #..#",
            b"##  #..#",
            b"#    #..#",
            b"     #..#",
            b"      ##",
        ];
        for (j, row) in CUR.iter().enumerate() {
            for (i, ch) in row.iter().enumerate() {
                let (px, py) = (x + i as i32, y + j as i32);
                if px >= 0 && py >= 0 && px < self.w && py < self.h && *ch != b' ' {
                    unsafe {
                        (self.fb.base_addr as *mut u32)
                            .add(py as usize * self.fb.stride as usize + px as usize)
                            .write_volatile(if *ch == b'#' { 0x202026 } else { 0xffffff });
                    }
                }
            }
        }
    }
}
const FONT: &[u8] = include_bytes!("assets/inter.bin");
const SIZES: [i32; 4] = [14, 16, 20, 25];
const EXTRA: &str = "·•…×←→↑↓°—–©№▶✓−÷⌘↻≡▦□⏻";
const COUNT: usize = 95 + 96 + 23;
fn glyph(ch: char, style: usize) -> (usize, i32, i32, i32, i32, i32) {
    let n = match ch {
        ' '..='~' => ch as usize - 32,
        '\u{400}'..='\u{45f}' => 95 + ch as usize - 0x400,
        c => EXTRA.chars().position(|v| v == c).map_or(31, |n| 191 + n),
    };
    let p = (style * COUNT + n) * 9;
    let d = &FONT[p..p + 9];
    (
        (u32::from_le_bytes(d[..4].try_into().unwrap()) as usize) + COUNT * 4 * 9,
        d[4] as i32,
        d[5] as i32,
        d[6] as i8 as i32,
        d[7] as i8 as i32,
        d[8] as i32,
    )
}
pub fn width(s: &str, style: usize) -> i32 {
    s.chars().map(|ch| glyph(ch, style).5).sum()
}
pub fn tail(s: &str, style: usize, max: i32) -> &str {
    let mut w = 0;
    for (i, c) in s.char_indices().rev() {
        w += glyph(c, style).5;
        if w > max {
            return &s[i + c.len_utf8()..];
        }
    }
    s
}
