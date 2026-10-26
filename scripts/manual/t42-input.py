#!/usr/bin/env python3
"""Bounded raw-input fixture for the T42 guided/remote checks."""
import os
import select
import sys
import termios
import time
import tty

path, mode = sys.argv[1:3]
setup = {
    "bracket": "\x1b[?2004h",
    "legacy": "",
    "mouse": "\x1b[?1003h\x1b[?1006h\x1b[?1004h",
    "kitty": "\x1b[>3u",
}[mode]
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
try:
    tty.setraw(fd)
    os.write(1, ("\x1b[2J\x1b[H" + setup + "Input probe: " + mode
                 + "\r\nLocal selection: alpha beta café 界😀"
                 + "\r\nCtrl+C ends this probe (or wait 5 minutes).\r\n").encode())
    with open(path, "wb", buffering=0) as log:
        log.write(b"READY\n")
        deadline = time.monotonic() + 300
        tail = b""
        while time.monotonic() < deadline:
            if not select.select([fd], [], [], max(0, deadline - time.monotonic()))[0]:
                break
            data = os.read(fd, 65536)
            if not data:
                break
            log.write(data)
            tail = (tail + data)[-32:]
            if b"\x03" in data or (mode == "kitty" and b"\x1b[99;5u" in tail):
                break
finally:
    os.write(1, b"\x1b[?2004l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[<5u\r\n")
    termios.tcsetattr(fd, termios.TCSANOW, old)
