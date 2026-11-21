#!/usr/bin/env python3
"""Rasterize the shipped Papirus vectors. The kernel loads the prebuilt atlas."""

import io
from pathlib import Path

import gi
from PIL import Image

gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf

ROOT = Path(__file__).resolve().parents[1]
NAMES = [
    "folder-violet",
    "org.gnome.Epiphany",
    "org.gnome.Console",
    "org.gnome.TextEditor",
    "org.gnome.Calculator",
    "org.gnome.SystemMonitor",
    "org.gnome.Settings",
    "gtk3-widget-factory",
    "org.gnome.Nibbles",
    "gnome-info",
    "folder-violet-documents",
    "folder-violet-download",
    "folder-violet-development",
]
SIZE = 96
data = bytearray()
for name in NAMES:
    path = ROOT / "third_party/papirus" / (name + ".svg")
    raster = GdkPixbuf.Pixbuf.new_from_file_at_scale(str(path), SIZE, SIZE, False)
    ok, png = raster.save_to_bufferv("png", [], [])
    if not ok:
        raise ValueError(f"Could not render {path}")
    img = (
        Image.open(io.BytesIO(png))
        .convert("RGBA")
        .quantize(colors=128, dither=Image.Dither.NONE)
    )
    indexes = img.tobytes()
    palette = bytearray(128 * 4)
    for index, rgba in zip(indexes, img.convert("RGBA").getdata()):
        palette[4 * index : 4 * index + 4] = bytes(rgba)
    data.extend(palette)
    data.extend(indexes)
(ROOT / "kernel/src/desktop/assets/icons.bin").write_bytes(data)
