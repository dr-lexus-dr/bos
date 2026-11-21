#!/usr/bin/env python3
"""Check all default bSH commands and both full/native interactive editors."""
from pathlib import Path
from qemu_console import boot

ROOT = Path(__file__).resolve().parents[1]
PROMPT = b"bSH ~ % "


def check_full_shell():
    # Explicit full-screen bSH profile. The graphical session is covered by test-live.
    with boot(ROOT / "target/esp", recovery=False, native=False, prompt=PROMPT, label="bsh-full-default") as (shell, output):
        assert b"All commands available" in output
        help_output = shell.command("help", b"commands:", prompt=PROMPT)
        for name in [b"ls", b"cd", b"pwd", b"mkdir", b"rm", b"cp", b"mv", b"babo", b"notepad", b"calculator", b"ifconfig", b"ping", b"mem", b"dmesg", b"runall", b"native"]:
            assert b"  " + name + b" " in help_output, name
        shell.command("pwd", b"/users/default", prompt=PROMPT)
        shell.command("mkdir -p documents/restored", prompt=PROMPT)
        shell.command("echo restored > documents/restored/a", prompt=PROMPT)
        shell.command("cp documents/restored/a documents/restored/b", prompt=PROMPT)
        shell.command("mv documents/restored/b documents/restored/c", prompt=PROMPT)
        shell.command("cat documents/restored/c", b"\r\nrestored\r\n", prompt=PROMPT)
        shell.command("cat documents/restored/c | wc -c", b"\r\n9\r\n", prompt=PROMPT)
        shell.command("echo abc | cat | wc -c > documents/count", prompt=PROMPT)
        shell.command("cat documents/count", b"\r\n4\r\n", prompt=PROMPT)
        shell.command('echo "a|b"', b"\r\na|b\r\n", prompt=PROMPT)
        shell.command("cd documents", prompt=b"bSH ~/documents % ")
        shell.command("pwd", b"/users/default/documents", prompt=b"bSH ~/documents % ")
        shell.command("cd", prompt=PROMPT)
        shell.command("calculator 2 + 3 * 4", b"\r\n14\r\n", prompt=PROMPT)
        shell.command("mem", b"phys ", prompt=PROMPT)
        shell.command("ifconfig", b"eth0", prompt=PROMPT)
        shell.command("ls documents/restored", b"c", prompt=PROMPT)
        shell.command("rm -r documents/restored", prompt=PROMPT)
        shell.command(b"pw\t", b"/users/default", prompt=PROMPT)
        shell.command(b"discard\x15echo kept", b"\r\nkept\r\n", prompt=PROMPT)
        shell.command("echo prefix", b"\r\nprefix\r\n", prompt=PROMPT)
        shell.command("help", b"commands:", prompt=PROMPT)
        shell.command(b"echo \x1b[A", b"\r\nprefix\r\n", prompt=PROMPT)
        shell.command("missing-command", b"unknown command", prompt=PROMPT)
        assert b"\x1b[91m% " in shell.raw
        shell.command("/applications/native-demo.bh", b"libbos native test passed", prompt=PROMPT)
        shell.command("native", b"Tab completes", prompt=PROMPT)
        shell.command("exit", prompt=PROMPT)  # leave the optional native shell
        shell.command("pwd", b"/users/default", prompt=PROMPT)
        shell.command("exit", b"bSH: new session", prompt=PROMPT)


def main():
    check_full_shell()
    with boot(ROOT / "target/esp", recovery=False, prompt=PROMPT, label="bsh-style") as (shell, output):
        assert b"bSH / bOS" in output
        assert b"\x1b[96mbSH " in shell.raw
        shell.command("echo hello", b"\r\nhello\r\n", prompt=PROMPT)
        shell.command(b"\x1b[A", b"\r\nhello\r\n", prompt=PROMPT)
        shell.command(b"echo ac\x1b[Db", b"\r\nabc\r\n", prompt=PROMPT)
        shell.command(b"he\t", b"bSH commands", prompt=PROMPT)
        shell.command("echo first", b"\r\nfirst\r\n", prompt=PROMPT)
        shell.command("help", b"bSH commands", prompt=PROMPT)
        shell.command(b"echo \x1b[A", b"\r\nfirst\r\n", prompt=PROMPT)
        shell.command(b"echo draft\x1b[A\x1b[B", b"\r\ndraft\r\n", prompt=PROMPT)
        shell.command(b"echo axc\x1b[D\x1b[D\x1b[3~b", b"\r\nabc\r\n", prompt=PROMPT)
        shell.command(b"discard\x15echo kept", b"\r\nkept\r\n", prompt=PROMPT)
        shell.command(b"echo bad\x17good", b"\r\ngood\r\n", prompt=PROMPT)
        shell.keys(b"/applications/native-\t")
        shell.expect(PROMPT)
        shell.command(b"\x15echo menu", b"\r\nmenu\r\n", prompt=PROMPT)
        long_text = "x" * 150
        shell.command("echo " + long_text, b"\r\n" + long_text.encode() + b"\r\n", prompt=PROMPT)
        shell.command("/missing.bh", b"bSH: command failed", prompt=PROMPT)
        assert b"\x1b[91m% " in shell.raw
        shell.command("/applications/native-demo.bh", b"libbos native test passed", prompt=PROMPT)
        shell.command("cat /users/default/documents/native.txt | wc -c", b"\r\n21\r\n", prompt=PROMPT)
        shell.command("exit", b"init: restarting shell", prompt=PROMPT)
        shell.keys(b"\x03")
        assert b"init: restarting shell" in shell.expect(PROMPT)
    print("bSH: all commands, default full shell, appearance, editing, history, completion, native pipes and restart passed")


if __name__ == "__main__":
    main()
