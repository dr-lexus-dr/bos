#!/usr/bin/env python3
"""Bake the native shell's Inter outlines into a compact coverage atlas."""
import struct
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

chars = list(range(32, 127)) + list(range(0x400, 0x460)) + list(map(ord, "·•…×←→↑↓°—–©№▶✓−÷⌘↻≡▦□⏻"))
records, pixels = bytearray(), bytearray()
for size, weight in ((14, 450), (16, 600), (20, 650), (25, 680)):
    font = ImageFont.truetype(sys.argv[1], size)
    axes = [axis["default"] for axis in font.get_variation_axes()]
    axes[-1] = weight
    font.set_variation_by_axes(axes)
    for cp in chars:
        ch = chr(cp)
        left, top, right, bottom = font.getbbox(ch, anchor="ls")
        w, h = right-left, bottom-top
        records.extend(struct.pack("<IBBbbB", len(pixels), w, h, left, top, round(font.getlength(ch))))
        if not w or not h:
            continue
        glyph = Image.new("L", (w, h))
        ImageDraw.Draw(glyph).text((-left, -top), ch, fill=255, font=font, anchor="ls")
        a = glyph.tobytes()
        pixels.extend((a[i] & 240) | (a[i+1] >> 4 if i+1 < len(a) else 0) for i in range(0,len(a),2))
Path(__file__).resolve().parents[1].joinpath("kernel/src/desktop/assets/inter.bin").write_bytes(records+pixels)
