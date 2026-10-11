#!/usr/bin/env python3
"""Resolve an emulator --profile histogram against an ELF program's symbols.

    tools/profsym.py PROFILE PROGRAM [N]
    tools/profsym.py --kernel PROFILE KERNEL [N]   (only kernel samples)

Prints the N (default 30) functions with the most samples. Kernel addresses
(below 0x40000000) are lumped together.
"""

import bisect
import struct
import sys


def symbols(path):
    if path.endswith(".map"):
        out = []
        for line in open(path):
            parts = line.split()
            if len(parts) >= 2:
                try:
                    out.append((int(parts[0], 16), parts[-1]))
                except ValueError:
                    pass
        out.sort()
        return out
    d = open(path, "rb").read()
    shoff, = struct.unpack_from(">I", d, 32)
    shentsize, shnum, shstrndx = struct.unpack_from(">HHH", d, 46)
    secs = [struct.unpack_from(">IIIIIIIIII", d, shoff + i * shentsize) for i in range(shnum)]
    out = []
    for s in secs:
        if s[1] != 2:  # SHT_SYMTAB
            continue
        strtab = secs[s[6]]
        for i in range(s[5] // 16):
            name_off, value, size, info, other, shndx = struct.unpack_from(">IIIBBH", d, s[4] + i * 16)
            if info & 0xF in (1, 2) or (info & 0xF == 0 and shndx != 0):
                end = d.index(b"\0", strtab[4] + name_off)
                name = d[strtab[4] + name_off:end].decode(errors="replace")
                if name and not name.startswith("."):
                    out.append((value, name))
    out.sort()
    return out


def main():
    args = sys.argv[1:]
    kernel = args[:1] == ["--kernel"]
    if kernel:
        args = args[1:]
    if len(args) < 2:
        sys.exit(__doc__)
    syms = symbols(args[1])
    n = int(args[2]) if len(args) > 2 else 30
    addrs = [a for a, _ in syms]
    counts = {}
    total = 0
    for line in open(args[0]):
        pc, c = line.split()
        pc, c = int(pc, 16), int(c)
        if kernel and pc >= 0x40000000:
            continue
        total += c
        if pc < 0x40000000 and not kernel:
            key = "[kernel]"
        else:
            i = bisect.bisect_right(addrs, pc) - 1
            key = syms[i][1] if i >= 0 else hex(pc)
        counts[key] = counts.get(key, 0) + c
    for k, c in sorted(counts.items(), key=lambda x: -x[1])[:n]:
        print(f"{100 * c / total:6.2f}%  {c:8d}  {k}")


if __name__ == "__main__":
    main()
