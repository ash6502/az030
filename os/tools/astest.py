#!/usr/bin/env python3
"""Differential test: assemble a generated corpus of 68030/68882/PMMU instructions with
azas and with GNU as (m68k-elf), and report every line whose encodings differ.

    python3 tools/astest.py [path/to/m68k-elf-as]

GNU binutils is only a reference for this test; the OS build does not use it.
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
GAS = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, ".toolchain/binutils/bin/m68k-elf-as")
AZAS_LINES = os.path.join(ROOT, "target/release/azas-lines")

# effective addresses by category
DN = ["d3"]
AN = ["a2"]
MEM = ["(a3)", "(a4)+", "-(a5)", "(12,a6)", "(-8,a0,d1.w)", "(4,a0,a1.l*4)", "(0x1234).w",
       "(0x12345678).l", "(1000,a0,d1.l*8)", "([4,a0])", "([4,a0],d1.w,8)", "([4,a0,d1.w*2],8)",
       "([0x1000])", "(d1.w*2)", "([a0],0x12345)", "(0x10000,a1)"]
PC = ["(100,pc)", "(10,pc,d0.l*2)", "(0x2000,pc,d1.w)", "([8,pc],d2.l)"]
IMM = {"b": "#0x5a", "w": "#0x1234", "l": "#0x12345678"}

ALL = DN + AN + MEM + PC
DATA = DN + MEM + PC
MEMORY = MEM + PC
CONTROL = [m for m in MEM if m not in ("(a4)+", "-(a5)")] + PC
ALTER = DN + AN + MEM
DATA_ALT = DN + MEM
MEM_ALT = MEM
CTRL_ALT = [m for m in MEM if m not in ("(a4)+", "-(a5)")]


def gen():
    L = []
    for op in ["add", "sub", "and", "or", "cmp"]:
        for sz in "bwl":
            for ea in DATA + (AN if op in ("add", "sub", "cmp") and sz != "b" else []):
                L.append(f"{op}.{sz} {ea},d5")
            if op != "cmp":
                for ea in MEM_ALT:
                    L.append(f"{op}.{sz} d5,{ea}")
            L.append(f"{op}.{sz} {IMM[sz]},d5")
            for ea in MEM_ALT:
                L.append(f"{op}i.{sz} {IMM[sz]},{ea}")
    for sz in "bwl":
        for ea in DATA_ALT:
            L.append(f"eor.{sz} d5,{ea}")
            L.append(f"eori.{sz} {IMM[sz]},{ea}")
    for op in ["adda", "suba", "cmpa"]:
        for sz in "wl":
            for ea in ALL:
                L.append(f"{op}.{sz} {ea},a3")
    for op in ["addq", "subq"]:
        for sz in "bwl":
            for ea in ALTER if sz != "b" else DATA_ALT:
                L.append(f"{op}.{sz} #3,{ea}")
            L.append(f"{op}.{sz} #8,d1")
    for op in ["addx", "subx", "abcd", "sbcd"]:
        for sz in ("bwl" if op.endswith("x") else "b"):
            L.append(f"{op}.{sz} d1,d2")
            L.append(f"{op}.{sz} -(a1),-(a2)")
    for sz in "bwl":
        L.append(f"cmpm.{sz} (a1)+,(a2)+")
    for op in ["ori", "andi", "eori"]:
        L.append(f"{op} #0x1f,ccr")
        L.append(f"{op} #0x2700,sr")
    # moves
    for sz in "bwl":
        for s in DATA + ([IMM[sz]] if True else []) + (AN if sz != "b" else []):
            for d in ["d2", "(a1)", "(a2)+", "-(a3)", "(8,a4)", "(2,a0,d0.l*2)", "(0x4000).w", "(0x123456).l", "([a0],4)"]:
                L.append(f"move.{sz} {s},{d}")
    for sz in "wl":
        for s in ALL:
            L.append(f"movea.{sz} {s},a1")
    L += ["moveq #-1,d3", "moveq #100,d7", "move.l a0,usp", "move.l usp,a6"]
    for ea in DATA:
        L.append(f"move.w {ea},sr")
        L.append(f"move.w {ea},ccr")
    for ea in DATA_ALT:
        L.append(f"move.w sr,{ea}")
        L.append(f"move.w ccr,{ea}")
    for sz in "wl":
        for ea in CTRL_ALT + ["-(a7)"]:
            L.append(f"movem.{sz} d0-d3/a0/a2-a4,{ea}")
        for ea in CONTROL + ["(a7)+"]:
            L.append(f"movem.{sz} {ea},d1/d5-d7/a6")
    L += ["movep.w (8,a1),d2", "movep.l (8,a1),d2", "movep.w d2,(8,a1)", "movep.l d2,(-4,a1)"]
    for sz in "bwl":
        for ea in MEM_ALT:
            L.append(f"moves.{sz} {ea},d1")
            L.append(f"moves.{sz} a2,{ea}")
    for cr in ["sfc", "dfc", "cacr", "usp", "vbr", "caar", "msp", "isp"]:
        L.append(f"movec {cr},d1")
        L.append(f"movec a3,{cr}")
    # single operand
    for op in ["clr", "neg", "negx", "not", "tst"]:
        for sz in "bwl":
            eas = DATA_ALT if op != "tst" else ALL + [IMM[sz]]
            if op == "tst" and sz == "b":
                eas = [e for e in eas if e not in AN]
            for ea in eas:
                L.append(f"{op}.{sz} {ea}")
    for ea in DATA_ALT:
        L += [f"nbcd {ea}", f"tas {ea}", f"st {ea}", f"sne {ea}", f"sls {ea}"]
    L += ["swap d4", "ext.w d2", "ext.l d2", "extb.l d2"]
    for ea in CONTROL:
        L += [f"pea {ea}", f"jmp {ea}", f"jsr {ea}", f"lea {ea},a4"]
    L += ["link a6,#-20", "link.l a6,#-100000", "unlk a6", "trap #15", "trap #0", "bkpt #3",
          "stop #0x2000", "rtd #8", "nop", "rts", "rte", "rtr", "reset", "trapv", "illegal",
          "trapne", "trapeq.w #5", "traplt.l #0x10000"]
    for ea in DATA:
        L += [f"chk.w {ea},d1", f"chk.l {ea},d1"]
    for sz in "bwl":
        for ea in CONTROL:
            L += [f"chk2.{sz} {ea},d3", f"cmp2.{sz} {ea},a3"]
        for ea in MEM_ALT:
            L.append(f"cas.{sz} d1,d2,{ea}")
    L += ["cas2.w d0:d1,d2:d3,(a0):(a1)", "cas2.l d0:d1,d2:d3,(d4):(a5)"]
    L += ["exg d1,d2", "exg a1,a2", "exg d1,a2", "exg a3,d4"]
    for ea in DATA:
        L += [f"mulu.w {ea},d1", f"muls.w {ea},d1", f"divu.w {ea},d1", f"divs.w {ea},d1",
              f"mulu.l {ea},d1", f"muls.l {ea},d2:d3", f"divu.l {ea},d1", f"divs.l {ea},d2:d3",
              f"divul.l {ea},d2:d3", f"divsl.l {ea},d4:d5"]
    L += ["pack d1,d2,#0x3030", "pack -(a1),-(a2),#0", "unpk d1,d2,#0x3030", "unpk -(a1),-(a2),#5"]
    # bits
    for op in ["btst", "bchg", "bclr", "bset"]:
        for ea in DATA_ALT:
            L += [f"{op} d1,{ea}", f"{op} #5,{ea}"]
    L += ["btst d1,(10,pc)", "btst #3,(10,pc)", "btst d0,#0x55"]
    for op in ["bftst", "bfchg", "bfclr", "bfset"]:
        for ea in ["d1", "(a0)", "(8,a1)", "(0x1000).l"]:
            L += [f"{op} {ea}{{4:8}}", f"{op} {ea}{{d2:d3}}", f"{op} {ea}{{0:32}}"]
    for op in ["bfextu", "bfexts", "bfffo"]:
        L += [f"{op} (a0){{2:5}},d4", f"{op} d1{{d0:7}},d5"]
    L += ["bfins d6,(a0){1:31}", "bfins d6,d1{d2:d3}"]
    # shifts
    for op in ["asl", "asr", "lsl", "lsr", "rol", "ror", "roxl", "roxr"]:
        for sz in "bwl":
            L += [f"{op}.{sz} #1,d2", f"{op}.{sz} #8,d2", f"{op}.{sz} d1,d2"]
        for ea in MEM_ALT:
            L.append(f"{op}.w {ea}")
    # FPU
    for op in ["fmove", "fint", "fsinh", "fintrz", "fsqrt", "flognp1", "fetoxm1", "ftanh", "fatan",
               "fasin", "fatanh", "fsin", "ftan", "fetox", "ftwotox", "ftentox", "flogn", "flog10",
               "flog2", "fabs", "fcosh", "fneg", "facos", "fcos", "fgetexp", "fgetman", "fdiv",
               "fmod", "fadd", "fmul", "fsgldiv", "frem", "fscale", "fsglmul", "fsub", "fcmp"]:
        L.append(f"{op}.x fp1,fp2")
        for sz, ea in [("l", "d1"), ("s", "(a0)"), ("x", "(8,a1)"), ("p", "(a2)+"), ("w", "d3"),
                       ("d", "(0x1000).l"), ("b", "(4,pc)"), ("l", "#0x12345678"), ("s", "#1.5"),
                       ("d", "#3.25"), ("x", "#-2.0"), ("w", "#7"), ("b", "#-3")]:
            L.append(f"{op}.{sz} {ea},fp3")
    for op in ["fsqrt", "fabs", "fneg", "fsin"]:
        L.append(f"{op}.x fp4")
    for sz, ea in [("l", "d1"), ("s", "(a0)"), ("x", "-(a1)"), ("w", "d3"), ("d", "(0x1000).l"), ("b", "(a2)+")]:
        L.append(f"fmove.{sz} fp5,{ea}")
    L += ["fsincos.x fp1,fp2:fp3", "fsincos.s (a0),fp4:fp5", "ftst.x fp2", "ftst.l d1", "ftst.d (a0)",
          "fmove.l d1,fpcr", "fmove.l fpsr,d2", "fmove.l (a0),fpiar", "fmove.l fpcr,(a1)",
          "fmovem.l fpcr/fpsr,-(a7)", "fmovem.l (a7)+,fpcr/fpsr/fpiar", "fmovem.l fpiar,d3",
          "fmovem.x fp0-fp7,-(a7)", "fmovem.x (a7)+,fp0-fp7", "fmovem.x fp2/fp4,(a0)",
          "fmovem.x (8,a0),fp1-fp3", "fmovem.x d1,-(a7)", "fmovem.x (a6)+,d2",
          "fmovecr.x #0x0f,fp1", "fmovecr.x #0x32,fp7", "fnop", "fsave -(a7)", "frestore (a7)+",
          "fsave (a0)", "frestore (8,a1)"]
    for c in ["f", "eq", "ogt", "oge", "olt", "ole", "ogl", "or", "un", "ueq", "ugt", "uge", "ult",
              "ule", "ne", "t", "sf", "seq", "gt", "ge", "lt", "le", "gl", "gle", "ngle", "ngl",
              "nle", "nlt", "nge", "ngt", "sne", "st"]:
        L += [f"fs{c} d1", f"fs{c} (a0)", f"ftrap{c}", f"ftrap{c}.w #1", f"ftrap{c}.l #2"]
    # PMMU (68030)
    for r, ea in [("tc", "(a0)"), ("srp", "(a1)"), ("crp", "(8,a2)"), ("tt0", "(a0)"), ("tt1", "(0x1000).l"), ("mmusr", "(a3)")]:
        L += [f"pmove {ea},{r}", f"pmove {r},{ea}"]
    L += ["pmovefd (a0),tc", "pmovefd (a1),srp", "pmovefd (a2),tt0",
          "pflusha", "pflush #1,#3", "pflush d2,#7", "pflush sfc,#0,(a0)", "pflush dfc,#4,(8,a1)",
          "ploadr #5,(a0)", "ploadw d3,(a1)", "ptestr #1,(a0),#7", "ptestw sfc,(a1),#3,a2",
          "ptestr d4,(0x1234).w,#0"]
    return L


def gas_listing(lines):
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "t.s")
        with open(src, "w") as f:
            for l in lines:
                f.write("\t" + l + "\n")
        lst = os.path.join(td, "t.lst")
        r = subprocess.run([GAS, "-m68030", "-m68882", "--register-prefix-optional", "-al=" + lst,
                            "-o", os.path.join(td, "t.o"), src], capture_output=True, text=True)
        errs = {}
        for e in r.stderr.splitlines():
            parts = e.split(":", 3)
            if len(parts) >= 3 and parts[1].isdigit():
                errs[int(parts[1])] = e
        out = {}
        for line in open(lst):
            if len(line) < 10 or line.startswith("68K GAS") or line.startswith("\f"):
                continue
            num = line[:4].strip()
            if not num.isdigit():
                continue
            n = int(num)
            addr = line[5:9].strip()
            hexpart = line[10:].split("\t")[0].split()
            if addr:
                out[n] = out.get(n, []) + hexpart
            else:
                out.setdefault(n, []).extend(hexpart)
        return out, errs


def azas_lines(lines):
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "t.s")
        with open(src, "w") as f:
            for l in lines:
                f.write("\t" + l + "\n")
        r = subprocess.run([AZAS_LINES, src], capture_output=True, text=True)
        out = {}
        for line in r.stdout.splitlines():
            n, rest = line.split(" ", 1)
            out[int(n)] = rest
        return out


def main():
    lines = gen()
    a = azas_lines(lines)
    bad = 0
    # GNU's listing columns only stay fixed for small files, so go in chunks
    for start in range(0, len(lines), 400):
        chunk = lines[start:start + 400]
        g, gerr = gas_listing(chunk)
        for j, l in enumerate(chunk, 1):
            i = start + j
            ga = " ".join(g.get(j, [])) if j not in gerr else "ERROR " + gerr[j]
            az = a.get(i, "")
            if ga.upper() != az.upper():
                bad += 1
                print(f"{l:45s} gas: {ga:40s} azas: {az}")
    print(f"{len(lines)} lines, {bad} differences")


if __name__ == "__main__":
    main()
