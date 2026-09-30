#!/usr/bin/env python3
"""Convert a raw binary to a $readmemh file: one big-endian 32-bit word per line,
padded to the ROM size (default 4096 words = 16 KB)."""
import sys
words = int(sys.argv[3]) if len(sys.argv) > 3 else 4096
data = open(sys.argv[1], "rb").read()
data += b"\0" * (-len(data) % 4)
assert len(data) // 4 <= words, "firmware larger than ROM"
with open(sys.argv[2], "w") as f:
    for i in range(0, len(data), 4):
        f.write(data[i:i+4].hex() + "\n")
    for _ in range(words - len(data) // 4):
        f.write("00000000\n")
