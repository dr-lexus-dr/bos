#!/usr/bin/env python3
"""Build the bootable BIOS/UEFI bOS Live disk."""

import lzma
import os
import struct
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/live"
SRC = ROOT / "bootloader/live"
ESP_LBA = 49
BROWSER = ROOT / "target/browser.bweb"


def run(*args):
    subprocess.run(args, cwd=ROOT, check=True)


def pack(raw):
    filters = [dict(id=lzma.FILTER_LZMA1, preset=9, dict_size=1 << 22)]
    return (
        b"\x5d"
        + struct.pack("<I", 1 << 22)
        + lzma.compress(raw, format=lzma.FORMAT_RAW, filters=filters)
    )


def decoder_objects(efi):
    target = "x86_64-unknown-windows" if efi else "i386-unknown-none"
    suffix = "efi.obj" if efi else "bios.o"
    sdk = ROOT / "third_party/lzma"
    objects = []
    for source in (SRC / "unpack.c", sdk / "LzmaDec.c"):
        obj = OUT / (source.stem + "-" + suffix)
        run(
            "clang",
            "--target=" + target,
            "-Oz",
            "-ffreestanding",
            "-fno-stack-protector",
            "-fno-pic",
            "-fno-unwind-tables",
            "-fno-asynchronous-unwind-tables",
            *([] if efi else ["-mno-sse", "-mno-sse2", "-mno-mmx", "-msoft-float"]),
            "-DZ7_LZMA_SIZE_OPT",
            "-I",
            SRC / "freestanding",
            "-I",
            sdk,
            "-c",
            source,
            "-o",
            obj,
        )
        objects.append(obj)
    return objects


def efi_partition(program, notice):
    """Small FAT12 ESP with contiguous files. Return the EFI file's sector."""
    sectors = lambda size: (size + 511) // 512
    cluster_sectors = 1
    while (sectors(len(program)) + sectors(len(notice))) // cluster_sectors > 4000:
        cluster_sectors *= 2
    if cluster_sectors > 64:
        raise ValueError("ESP exceeds supported FAT12 cluster size")
    cluster_bytes = cluster_sectors * 512
    clusters = lambda n: (n + cluster_bytes - 1) // cluster_bytes
    file_clusters = clusters(len(program))
    notice_clusters = clusters(len(notice))
    used = 2 + file_clusters + notice_clusters  # EFI and BOOT directories
    fat_sectors = sectors(((used + 2) * 3 + 1) // 2)
    data_lba = 1 + 2 * fat_sectors + 1  # BPB, mirrored FATs, 16 root entries
    total = data_lba + used * cluster_sectors
    boot = bytearray(512)
    boot[:11] = b"\xeb\x3c\x90bOS Live"
    struct.pack_into(
        "<HBHBHHBHHHII",
        boot,
        11,
        512,
        cluster_sectors,
        1,
        2,
        16,
        total if total <= 65535 else 0,
        0xF8,
        fat_sectors,
        32,
        64,
        ESP_LBA,
        total if total > 65535 else 0,
    )
    struct.pack_into(
        "<BBBI11s8s", boot, 36, 0x80, 0, 0x29, 0x624F5301, b"BOS LIVE   ", b"FAT12   "
    )
    boot[510:] = b"\x55\xaa"
    fat = bytearray(512 * fat_sectors)
    entries = [0xFF8, 0xFFF, 0xFFF, 0xFFF]
    for size in (file_clusters, notice_clusters):
        first = len(entries)
        entries.extend([*range(first + 1, first + size), 0xFFF])
    for i, value in enumerate(entries):
        offset = i * 3 // 2
        value <<= (i & 1) * 4
        fat[offset] |= value & 255
        fat[offset + 1] |= value >> 8

    def entry(name, directory, cluster, size=0):
        result = bytearray(32)
        result[:11] = name.encode("ascii").ljust(11)
        result[11] = 16 if directory else 32
        # Reproducible FAT timestamp, 1980-01-01 at midnight.
        for offset in (16, 18, 24):
            struct.pack_into("<H", result, offset, 0x21)
        struct.pack_into("<HI", result, 26, cluster, size)
        return result

    label = entry("BOS LIVE", False, 0)
    label[11] = 0x08
    root = (
        label
        + entry("EFI", True, 2)
        + entry("NOTICES TXT", False, 4 + file_clusters, len(notice))
    )
    efi = entry(".", True, 2) + entry("..", True, 0) + entry("BOOT", True, 3)
    folder = (
        entry(".", True, 3)
        + entry("..", True, 2)
        + entry("BOOTX64 EFI", False, 4, len(program))
    )
    header = bytes(boot + fat + fat + root.ljust(512, b"\0"))
    files = b"".join(
        bytes(p).ljust(clusters(len(p)) * cluster_bytes, b"\0")
        for p in [efi, folder, program, notice]
    )
    return header + files, data_lba + 2 * cluster_sectors


def section_offset(pe, name):
    header = struct.unpack_from("<I", pe, 0x3C)[0]
    count = struct.unpack_from("<H", pe, header + 6)[0]
    size = struct.unpack_from("<H", pe, header + 20)[0]
    for i in range(count):
        sec = header + 24 + size + 40 * i
        if pe[sec : sec + 8].rstrip(b"\0") == name:
            return struct.unpack_from("<I", pe, sec + 20)[0]
    raise ValueError(f"missing PE section {name!r}")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    run("bash", "scripts/build-browser.sh")
    run(
        "cargo",
        "rustc",
        "--locked",
        "-p",
        "bos_kernel",
        "--target",
        "x86_64-unknown-none",
        "--profile",
        "live",
        "--features",
        "live",
        "--",
        "-C",
        "relocation-model=static",
        "-C",
        "link-arg=-Tbootloader/live/link.ld",
    )
    kernel = ROOT / "target/x86_64-unknown-none/live/bos_kernel"
    run("objcopy", "-O", "binary", kernel, OUT / "kernel.bin")
    raw = (OUT / "kernel.bin").read_bytes()
    names = subprocess.check_output(["nm", "-n", kernel], text=True)
    symbols = {
        fields[-1]: int(fields[0], 16)
        for line in names.splitlines()
        if len(fields := line.split()) == 3
    }
    if symbols["live_entry"] != 0x100000 or symbols["__file_end"] != 0x100000 + len(
        raw
    ):
        raise ValueError("flat binary does not match the loader map")
    stream = pack(raw)
    browser = BROWSER.read_bytes()
    payload = stream + browser
    packed = (
        struct.pack(
            "<4sIIII",
            b"bOS3",
            len(raw),
            len(stream),
            symbols["__kernel_end"] - 0x100000,
            len(browser),
        ).ljust(512, b"\0")
        + payload
    )
    blocks = OUT / "blocks.bin"
    blocks.write_bytes(packed)
    efi_objects = decoder_objects(True)
    subprocess.run(
        [
            "cargo",
            "rustc",
            "--locked",
            "-p",
            "bloader",
            "--bin",
            "live-efi",
            "--features",
            "live-efi",
            "--target",
            "x86_64-unknown-uefi",
            "--release",
            "--",
            *(flag for obj in efi_objects for flag in ("-C", f"link-arg={obj}")),
        ],
        cwd=ROOT,
        env={**os.environ, "BOS_LIVE_BLOCKS": str(blocks)},
        check=True,
    )
    pe = (ROOT / "target/x86_64-unknown-uefi/release/live-efi.efi").read_bytes()
    offset = section_offset(pe, b".bos")
    if offset % 512 or pe[offset : offset + len(packed)] != packed:
        raise ValueError("EFI payload alignment or content mismatch")
    esp, file_sector = efi_partition(
        pe, (ROOT / "target/browser-notices.txt").read_bytes()
    )
    payload_lba = ESP_LBA + file_sector + offset // 512 + 1
    run("as", "--32", SRC / "mbr.S", "-o", OUT / "mbr.o")
    run(
        "ld",
        "-m",
        "elf_i386",
        "-Ttext",
        "0x7c00",
        "--oformat",
        "binary",
        OUT / "mbr.o",
        "-o",
        OUT / "mbr.bin",
    )
    run(
        "as",
        "--32",
        "--defsym",
        f"KERNEL_END={symbols['__kernel_end']}",
        "--defsym",
        f"FILE_END={symbols['__file_end']}",
        "--defsym",
        f"PAYLOAD_LBA={payload_lba}",
        "--defsym",
        f"PACKED_SIZE={len(stream)}",
        "--defsym",
        f"PAYLOAD_SIZE={len(payload)}",
        SRC / "stage.S",
        "-o",
        OUT / "stage.o",
    )
    bios_objects = decoder_objects(False)
    layout = OUT / "stage.ld"
    layout.write_text(
        "SECTIONS { . = 0x8000; .text : { *(.text.start) *(.text*) *(.rodata*) *(.data*) } }"
    )
    run(
        "ld",
        "-m",
        "elf_i386",
        "-T",
        layout,
        "--oformat",
        "binary",
        OUT / "stage.o",
        *bios_objects,
        "-o",
        OUT / "stage.bin",
    )
    mbr = bytearray((OUT / "mbr.bin").read_bytes())
    stage = (OUT / "stage.bin").read_bytes()
    if len(mbr) != 512 or len(stage) > (ESP_LBA - 1) * 512:
        raise ValueError("boot stages exceed their reserved sectors")
    struct.pack_into(
        "<B3sB3sII",
        mbr,
        446,
        0x80,
        b"\x00\x09\x00",
        0xEF,
        b"\xfe\xff\xff",
        ESP_LBA,
        len(esp) // 512,
    )
    disk = mbr + stage.ljust((ESP_LBA - 1) * 512, b"\0") + esp
    temp = OUT / "bos-live.img.tmp"
    temp.write_bytes(disk)
    os.replace(temp, OUT / "bos-live.img")
    print(
        f"{OUT}/bos-live.img  {len(disk):,} bytes; resident kernel "
        f"{symbols['__kernel_end'] - 0x100000:,} bytes including BSS and stack"
    )


if __name__ == "__main__":
    main()
