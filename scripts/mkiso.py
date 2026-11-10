#!/usr/bin/env python3
"""bOS live ISO builder (no root needed).
Pipeline:
  1. read the ESP staged by scripts/build.sh
  2. mkfs.vfat + mtools mcopy -> efiboot.img (FAT)
  3. pycdlib ISO with El Torito EFI boot entry
Usage: python3 scripts/mkiso.py [--install] [--esp target/esp] [--out IMAGE]
"""
import argparse, os, pathlib, subprocess, sys

MTOOLS = pathlib.Path(os.environ.get("MTOOLS_DIR", ""))
ESP_FILES = ("EFI/BOOT/BOOTX64.EFI", "EFI/bOS/kernel.bin", "EFI/bOS/initrd.img")

def sh(*args):
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        print(f"CMD FAILED: {' '.join(args)}\n{r.stderr}", file=sys.stderr)
        sys.exit(1)
    return r

def make_fat(esp: pathlib.Path, fat_img: pathlib.Path):
    # Leave room for FAT metadata and directory entries.
    size_mib = max(16, sum((esp / name).stat().st_size for name in ESP_FILES) // (1 << 20) + 4)
    with fat_img.open("wb") as image:
        image.truncate(size_mib << 20)
    sh("mkfs.vfat", "-F", "16", "-n", "BOS_EFI", str(fat_img))
    for directory in ("EFI", "EFI/BOOT", "EFI/bOS"):
        sh(str(MTOOLS / "mmd"), "-i", str(fat_img), f"::{directory}")
    for name in ESP_FILES:
        sh(str(MTOOLS / "mcopy"), "-i", str(fat_img), str(esp / name), f"::{name}")
    print(f"FAT image: {fat_img} ({fat_img.stat().st_size} bytes)")

def mkiso(esp: pathlib.Path, fat_img: pathlib.Path, out: pathlib.Path, install_files):
    from pycdlib import PyCdlib
    iso = PyCdlib()
    iso.new(joliet=3, rock_ridge="1.12", sys_ident="bOS", vol_ident="BOS_INSTALL" if install_files else "BOS_LIVE")
    for d, rr in [("/EFI", "EFI"), ("/EFI/BOOT", "BOOT"), ("/EFI/BOS", "bOS")]:
        iso.add_directory(d, rr_name=rr, joliet_path=d)
    for name in ESP_FILES:
        path = esp / name
        iso_path = "/" + name.upper()
        iso.add_file(str(path), iso_path, rr_name=path.name, joliet_path=iso_path)
    for source, name in install_files:
        iso.add_file(str(source), "/" + name.upper(), rr_name=name, joliet_path="/" + name)
    # El Torito EFI boot image inside ISO too
    iso.add_file(str(fat_img), "/EFIBOOT.IMG",
                 rr_name="efiboot.img", joliet_path="/efiboot.img")
    iso.add_eltorito("/EFIBOOT.IMG", "/BOOT.CAT", efi=True)
    iso.write(str(out))
    iso.close()

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out")
    ap.add_argument("--install", action="store_true", help="include the Linux disk installer and guide")
    ap.add_argument("--esp", default="target/esp")
    a = ap.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    esp = root / a.esp
    for name in ESP_FILES:
        if not (esp / name).is_file():
            ap.error(f"missing {esp / name} — run scripts/build.sh first")
    fat_img = root / "target" / "efiboot.img"
    fat_img.parent.mkdir(parents=True, exist_ok=True)
    make_fat(esp, fat_img)
    install_files = []
    if a.install:
        install_files = [(root / "scripts/mkusb.sh", "install.sh"), (root / "docs/INSTALL.md", "INSTALL.md")]
    out = root / (a.out or ("target/bos-install.iso" if a.install else "target/bos-live.iso"))
    out.parent.mkdir(parents=True, exist_ok=True)
    mkiso(esp, fat_img, out, install_files)
    print(f"ISO: {out} ({out.stat().st_size} bytes)")

if __name__ == "__main__":
    sys.exit(main())
