#!/usr/bin/env python3
"""Integration checks for the real upstream host userspace (build first)."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
profile = os.environ.get("BOS_PROFILE", "debug")
bin_dir = root / "target" / profile
shell = bin_dir / "bsh"
utils = bin_dir / "bcoreutils"
with tempfile.TemporaryDirectory(prefix="bos-upstream-") as directory:
    def run(*args):
        return subprocess.run(args, cwd=directory, text=True, capture_output=True)

    result = run(str(shell), "--norc", "-c",
                 "printf 'one\\ntwo\\n' > input; cat input | wc -l")
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "2", result.stdout
    result = run(str(shell), "--norc", "-c", "exit 23")
    assert result.returncode == 23, result
    result = run(str(utils), "cat", "input")
    assert result.returncode == 0 and result.stdout == "one\ntwo\n", result
    result = run(str(bin_dir / "host/bin/cat"), "input")
    assert result.returncode == 0 and result.stdout == "one\ntwo\n", result
    result = run(str(utils), "cat", "missing")
    assert result.returncode != 0 and result.stderr, result
    result = run(str(shell), "--norc", "-c", "ls --version")
    assert result.returncode == 0 and "uutils" in result.stdout, result
print("nash + uutils host integration: OK")
