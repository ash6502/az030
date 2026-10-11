# AZ/UX status

## Working
- Toolchain: rustc with a patched LLVM `llc` (`toolchain/llvm-m68k-fixes.patch`,
  six M68k back-end fixes); `make cgtest` checks the code generator against the host.
- Kernel: PMMU address spaces, processes, signals, sessions/job control, pipes,
  tty with line discipline, SCSI disks, buffer cache (O(1) LRU), file system,
  100 Hz timer, kernel log (`dmesg`).
- Userland: `init`, `login`, `sh`, ~90 utilities, `edit`, `make`, `man`, manual
  pages in `/usr/share/man`.
- Native toolchain on AZ/UX: `cc` (azcc C99 compiler), `as`, `ld`, `ar`; libc with
  stdio, stdlib, string, math (68882), time, unistd, signals, setjmp, dirent,
  termios, pwd/grp, getopt.
- Tests: `tests/c/run.sh` (differential C tests: host compiler vs azcc on AZ/UX),
  `tests/os/*.t` (scripted emulator sessions), `make cgtest`.

## Known limitations
- An M68k back-end miscompile remains: in azcc's `Parser::stmt`, a
  `for x in a..=b` loop over i64 never terminated (worked around with an explicit
  counter; a small reproduction in cgtest does not trigger it).
- Compiling on AZ/UX is slow: about 15 s for a 500-line file on the emulated
  25 MHz machine. Most of the time is the assembler's text processing, then the
  heap and memory copies (profile with `az030-emu --profile`).
- azcc: no complex numbers, variable-length arrays or wide-character library
  functions; `asm()` statements take no operands. Arguments are evaluated right
  to left.
- One timezone (UTC); no networking; one console terminal.
- `rm` cannot remove the current directory (the kernel refuses with EBUSY).

## Not planned for now
- The Rust-subset compiler (made optional).
