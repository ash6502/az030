#!/usr/bin/env python3
"""Look for a classic m68k code-generation bug in a disassembly: a conditional
branch (or Scc) whose condition codes were last set by a data-register move or a
constant load, which is almost never what the compiler meant (it usually means a
copy was scheduled between a compare and its branch).

    m68k-elf-objdump -d prog.o | python3 tools/flagcheck.py

Only straight-line code is examined (the scan stops at labels and branches), so
this finds candidates, not proofs.
"""
import re
import sys

COND = re.compile(r'^(b(hi|ls|cc|cs|ne|eq|vc|vs|pl|mi|ge|lt|gt|le)[sw]?|s(hi|ls|cc|cs|ne|eq|vc|vs|pl|mi|ge|lt|gt|le))$')
# instructions that leave CCR alone
KEEP = re.compile(r'^(movea[wl]?|moveml|movemw|lea|pea|bra[sw]?|jmp|jsr|bsr[sw]?|exg|link|unlk|adda[wl]?|suba[wl]?|nop|rts|movew %sp@\+,%ccr|movew %ccr,%sp@-)')
SUSPECT = re.compile(r'^(movel|movew|moveb|moveq|clrl|clrw|clrb)\s')


def main():
    lines = [l.rstrip('\n') for l in sys.stdin]
    insns = []
    func = '?'
    for l in lines:
        m = re.match(r'^[0-9a-f]+ <(.*)>:', l)
        if m:
            func = m.group(1)
            insns.append(None)
            continue
        parts = l.split('\t')
        if len(parts) >= 3 and re.match(r'^\s*[0-9a-f]+:$', parts[0]):
            insns.append((func, parts[0].strip().rstrip(':'), parts[2].strip()))
    hits = 0
    for i, ins in enumerate(insns):
        if ins is None:
            continue
        mnem = ins[2].split()[0] if ins[2] else ''
        if not COND.match(mnem):
            continue
        j = i - 1
        while j >= 0 and insns[j] is not None:
            text = insns[j][2]
            m0 = text.split()[0] if text else ''
            if text.startswith('movew %sp@+,%ccr'):
                break  # flags explicitly restored
            if KEEP.match(text) or (m0.startswith('move') and text.endswith(('%a0', '%a1', '%a2', '%a3', '%a4', '%a5', '%fp', '%sp'))):
                j -= 1
                continue
            if SUSPECT.match(text) and not text.endswith(',%sp@-'):
                # stores to memory set flags too, but a register move right before
                # a branch is the suspicious case
                if re.search(r',%d[0-7]$', text) or m0.startswith('clr') or m0 == 'moveq':
                    hits += 1
                    print(f'{ins[0]} @{ins[1]}: {ins[2]}  <- flags from @{insns[j][1]}: {text}')
            break
    print(f'{hits} suspicious branch(es)', file=sys.stderr)


if __name__ == '__main__':
    main()
