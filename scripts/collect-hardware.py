#!/usr/bin/env python3
"""Collect read-only Linux hardware evidence for the target laptop.
Run on the E7270, not in a VM. No sudo, mounting, or disk writes are performed
except writing the requested report directory. Inaccessible sources are recorded.
"""
import argparse
import datetime
import json
from pathlib import Path
import shutil
import subprocess

COMMANDS = {
    "uname": ["uname", "-a"],
    "cpu": ["lscpu", "--json"],
    "pci": ["lspci", "-nnk"],
    "usb": ["lsusb"],
    "usb-tree": ["lsusb", "-t"],
    "disks": ["lsblk", "--json", "-o", "NAME,TYPE,SIZE,MODEL,TRAN,FSTYPE,MOUNTPOINTS"],
    "input": ["cat", "/proc/bus/input/devices"],
    "iomem": ["cat", "/proc/iomem"],
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    report = {"created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "commands": {}, "dmi": {}, "errors": []}
    for label, command in COMMANDS.items():
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=30)
            report["commands"][label] = {"argv": command, "exit": result.returncode,
                                          "stdout": result.stdout, "stderr": result.stderr}
        except (OSError, subprocess.TimeoutExpired) as error:
            report["commands"][label] = {"error": str(error)}
    for name in ("sys_vendor", "product_name", "product_version", "bios_version", "bios_date"):
        try:
            report["dmi"][name] = (Path("/sys/class/dmi/id") / name).read_text().strip()
        except OSError as error:
            report["errors"].append(f"DMI {name}: {error}")
    tables = args.out / "acpi"
    tables.mkdir()
    source = Path("/sys/firmware/acpi/tables")
    try:
        for table in sorted(source.iterdir()):
            if table.is_file():
                try:
                    shutil.copyfile(table, tables / table.name)
                except OSError as error:
                    report["errors"].append(f"ACPI {table.name}: {error}")
    except OSError as error:
        report["errors"].append(f"ACPI inventory: {error}")
    (args.out / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(args.out / "report.json")

if __name__ == "__main__":
    main()
