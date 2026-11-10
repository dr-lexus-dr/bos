#!/usr/bin/env python3
"""Generate kernel/src/drivers/font8x16.rs from a PSF console font.
Reads PSF v1/v2 (e.g. /usr/share/consolefonts/Lat15-VGA16.psf.gz),
emits glyphs 0x00-0xFF as 16 bytes each (8x16, MSB = left pixel).
Usage: python3 scripts/genfont.py [psf.gz] [out.rs]
Only ASCII 0x20-0x7E is used by bSH; the rest is kept for completeness.
"""
import gzip, pathlib, struct, sys

SRC = "/usr/share/consolefonts/Lat15-VGA16.psf.gz"

def load(path):
    raw = gzip.open(path, "rb").read()
    if raw[:2] == b"\x36\x04":  # PSF1
        n = 256 if raw[2] == 0 else 512
        h = raw[3]
        return n, 8, h, raw[4:4 + n * h]
    if raw[:4] == b"\x72\xb5\x4a\x86":  # PSF2
        _ver, _hsz, _flags, n, charsize, h, w = struct.unpack("<7I", raw[:28])
        assert w == 8 and h == 16, f"need 8x16, got {w}x{h}"
        assert charsize == 16
        return n, w, h, raw[32:32 + n * charsize]
    raise SystemExit("unknown PSF magic")

def main():
    src = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else SRC)
    out = pathlib.Path(sys.argv[2] if len(sys.argv) > 2
                        else "kernel/src/drivers/font8x16.rs")
    n, w, h, data = load(src)
    assert n >= 256, f"need >=256 glyphs, got {n}"
    L = [f"//! 8x16 console font generated from {src.name} — do not edit.",
         "//! Glyph i = FONT[i], 16 rows, MSB is the left pixel.",
         "#[allow(dead_code)]",
         "pub const FONT_W: usize = 8;",
         "#[allow(dead_code)]",
         "pub const FONT_H: usize = 16;",
         "pub const FONT: [[u8; 16]; 256] = ["]
    for g in range(256):
        rows = ", ".join(f"0x{b:02x}" for b in data[g * 16:(g + 1) * 16])
        L.append(f"    [{rows}], // 0x{g:02x}")
    L.append("];")
    out.write_text("\n".join(L) + "\n")
    print(f"wrote {out} ({n} glyphs 8x16 from {src.name})")

if __name__ == "__main__":
    main()
