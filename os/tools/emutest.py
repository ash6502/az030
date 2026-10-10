#!/usr/bin/env python3
"""Drive the OS in the emulator from a script, expect-style.

    tools/emutest.py [-c emu.toml] [-t timeout] SCRIPT

SCRIPT lines:
    > text        wait for a shell prompt ('# ' or '$ ' at the end of the output),
                  then send text + newline
    = text        send text + newline immediately
    ? regex       wait until the output matches regex (since the last send)
    ! regex       fail if the output so far matches regex
    login NAME    wait for 'login: ' and send NAME
    key X         send a control key (X = C, D, Z, ...) or 'esc'
    # ...         comment

Everything the emulator prints is copied to stdout (carriage returns removed).
Exits 0 when the script completes, 1 on a timeout or failed check.
"""

import os
import re
import select
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
OS = os.path.dirname(HERE)
EMU = os.path.join(os.path.dirname(OS), "emu", "target", "release", "az030-emu")
PROMPT = re.compile(r"[#$] $")
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


class Emu:
    def __init__(self, config, timeout):
        self.p = subprocess.Popen([EMU, "-c", config], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.out = b""
        self.mark = 0
        self.timeout = timeout

    def pump(self, wait):
        r, _, _ = select.select([self.p.stdout], [], [], wait)
        if r:
            data = os.read(self.p.stdout.fileno(), 65536)
            if not data:
                raise EOFError("emulator exited")
            self.out += data
            sys.stdout.write(data.decode("utf-8", "replace").replace("\r", ""))
            sys.stdout.flush()

    def text(self):
        return ANSI.sub("", self.out[self.mark:].decode("utf-8", "replace").replace("\r", ""))

    def wait(self, pred, what):
        deadline = time.time() + self.timeout
        while not pred(self.text()):
            if time.time() > deadline:
                raise TimeoutError(f"timed out waiting for {what}")
            self.pump(0.1)

    def send(self, data):
        self.mark = len(self.out)
        self.p.stdin.write(data)
        self.p.stdin.flush()

    def close(self):
        try:
            self.p.stdin.close()
        except OSError:
            pass
        end = time.time() + 2
        while time.time() < end:
            try:
                self.pump(0.1)
            except EOFError:
                break
        self.p.kill()


def main():
    args = sys.argv[1:]
    config = os.path.join(OS, "build", "test.toml")
    timeout = 60.0
    while args and args[0].startswith("-"):
        if args[0] == "-c":
            config = args[1]
            args = args[2:]
        elif args[0] == "-t":
            timeout = float(args[1])
            args = args[2:]
        else:
            sys.exit(__doc__)
    if len(args) != 1:
        sys.exit(__doc__)
    lines = open(args[0]).read().splitlines()
    emu = Emu(config, timeout)
    ok = True
    try:
        for line in lines:
            if not line.strip() or line.startswith("#"):
                continue
            cmd, _, arg = line.partition(" ")
            if cmd == ">":
                emu.wait(lambda t: PROMPT.search(t), "a prompt")
                emu.send(arg.encode() + b"\n")
            elif cmd == "=":
                emu.send(arg.encode() + b"\n")
            elif cmd == "?":
                rx = re.compile(arg, re.M)
                emu.wait(lambda t: rx.search(t), repr(arg))
            elif cmd == "!":
                if re.search(arg, emu.text(), re.M):
                    raise AssertionError(f"output matched {arg!r}")
            elif cmd == "login":
                emu.wait(lambda t: t.endswith("login: "), "the login prompt")
                emu.send(arg.encode() + b"\n")
            elif cmd == "key":
                k = b"\x1b" if arg == "esc" else bytes([ord(arg.upper()) & 0x1F])
                emu.send(k)
            else:
                raise ValueError(f"bad script line: {line}")
        emu.wait(lambda t: PROMPT.search(t), "the final prompt")
    except (TimeoutError, AssertionError, EOFError) as e:
        print(f"\n*** emutest: {e}", file=sys.stderr)
        ok = False
    finally:
        emu.close()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
