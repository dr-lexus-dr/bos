#!/usr/bin/env python3
"""Exercise real nash/uutils in the booted bOS kernel (not on the host)."""
from pathlib import Path
from qemu_console import boot
ROOT = Path(__file__).resolve().parents[1]
CASES = [
    ("bfetch", b"OS:     bOS"),
    ("bpt --version", b"bpt v0.1.0"),
    ("stat /bin/nash", b"File:"),
    ("id -u", b"0"),
    ("printf abc | wc -c", b"3"),
    ("sleep 0.02; echo SLEPT", b"SLEPT"),
    ("umask 077; printf x > /tmp/private; ls -l /tmp/private; umask 022", b"rw-------"),
    ("mkdir -p /tmp/uu; printf 'beta\\nalpha\\n' > /tmp/uu/input; cat /tmp/uu/input", b"alpha"),
    ('cp /tmp/uu/input /tmp/uu/copy; test "$(cat /tmp/uu/input)" = "$(cat /tmp/uu/copy)"; echo status=$?', b"status=0"),
    ("mv /tmp/uu/copy /tmp/uu/moved; ls /tmp/uu", b"moved"),
    ("head -n 1 /tmp/uu/input; tail -n 1 /tmp/uu/input", b"alpha"),
    ("printf abc | sha256sum", b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
    ("/bin/nash -c 'exit 23'; echo status=$?", b"status=23"),
    ("(cd /tmp; pwd); pwd", b"/users/default"),
    ("rm /tmp/uu/moved; test ! -e /tmp/uu/moved; echo status=$?", b"status=0"),
]
with boot(ROOT / 'target/esp', recovery=False, native=False, profile="nash",
          prompt=b'nash> ', label='native-upstream') as (shell, output):
    assert b'init: PID 1 in ring 3' in output, output[-2000:]
    # Disable the interactive editor so prompts cannot satisfy output assertions.
    shell.keys(b"PS1=''; set +o emacs\r")
    for index, (command, expected) in enumerate(CASES):
        marker = f'BOS_RESULT_{index}'
        shell.keys((command + "; printf '\\nBOS_RESULT_%s\\n' " + str(index) + '\r').encode())
        response = shell.expect(('\r\n' + marker + '\r\n').encode(), timeout=60)
        assert expected in response, (command, response[-3000:])
        assert b'user fault' not in response, response[-3000:]
        print('PASS:', command, flush=True)
