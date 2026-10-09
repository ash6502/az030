# az030 OS: work in progress

## Done and verified
- `lib/azas`: 68030/68882/PMMU assembler (vasm-style syntax, binary + ELF output).
  Assembles `rom/boot.bin` byte-for-byte; matches GNU as on a ~4000-instruction corpus
  (`tools/astest.py`).
- `lib/azld`: static ELF linker (archives/rlibs, GC sections, ELF exec or flat binary);
  compiles LLVM bitcode with a patched `llc` (`tools/src/bin/azld.rs`).
- `lib/rtasm`: asm runtime (mem*, 64-bit int helpers, FPU-backed float ABI) replacing
  compiler_builtins, which rustc compiles with its unpatched LLVM.
- Toolchain: local rust-src + LLVM 23.1.1 llc with M68k fixes (`toolchain/llvm-m68k-fixes.patch`):
  1. store of an extending load folded into a full-width mem-to-mem move
  2. frame-index + index register addressing dropped the index
  3. register copies clobbered CCR between compare and branch
  4. fastcc sret: callee popped a stack slot that was never pushed
- emulator: timer/IRQ block at 0xFE004000, `--trace`, `--stop-at`, `--dump`.
- `boot/boot.s` bootloader, `tools/src/bin/azmkdisk.rs` disk/azfs builder, `lib/azfs`, `lib/azsys`.

## In progress
- One known remaining codegen bug: Grisu exact float formatting (`{:.3}`) wrong.
  `llc -verify-machineinstrs` reports errors (illegal vregs for `MOVM16*_P` spills of DR8
  values, allocatable live-ins, undefined physregs, `AND32di` on address registers) —
  next lead.
- Kernel (`kernel/`): written and type-checks, never built/run yet.

## Not started
- Rust userland runtime + utilities, shell, nano-like editor, C / Rust-subset compilers,
  Makefile, disk image assembly, docs.
