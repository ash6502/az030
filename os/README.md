# AZ/UX

AZ/UX is a small UNIX-like operating system for the az030, a 68030 computer
with a 68882 FPU, SCSI disks and a serial console (see `../SPECS.txt` and
`../MEM_MAP.md`). It boots from a SCSI disk and runs in the emulator in
`../emu`.

What you get:

- A preemptive multitasking kernel with protected address spaces (68030
  PMMU), a UNIX file system on SCSI disks, pipes, signals, sessions and job
  control, users and permissions.
- `init`, `login` and a POSIX-style shell with job control and line editing.
- About 90 utilities: the file, text and system tools you would expect
  (`ls cp mv rm find grep sed sort diff more ps kill df du mount su passwd
  ...`), plus `edit` (a nano-style editor), `make` and `man`.
- A native toolchain that runs on AZ/UX itself: `cc` (a C99 compiler),
  `as` (68030/68882 assembler), `ld` (linker) and `ar`, with a C library in
  `/usr/lib` and headers in `/usr/include`.

## Building

Everything is built on the host with the Rust toolchain; `make` drives it.

```
make toolchain   # once: fetch rust-src and build the patched LLVM llc (~1 hour)
make             # kernel, bootloader, programs, libc and build/disk.img
make run         # boot it in the emulator (log in as root, no password)
```

`DISK_SIZE` (default `64M`) sets the image size and `CMDLINE` the kernel
command line. The emulator console exits with Ctrl-] q.

## Testing

```
make cgtest                       # Rust code generator: host vs 68030 output
tests/c/run.sh                    # C compiler + libc: host cc vs azcc on AZ/UX
tools/emutest.py tests/os/shell.t # scripted sessions in the emulator
```

`tests/os/*.t` cover the shell (`shell.t`), the utilities (`utils.t`), the
editor (`edit.t`), the assembler and linker (`asm.t`) and the C compiler
(`cc.t`). `tools/emutest.py` boots `build/disk.img`, logs in and drives
the console from a script (see the comment at the top of it).

For performance work, `az030-emu --profile FILE` samples the program
counter and `tools/profsym.py` turns the samples into a per-function
report.

## Source layout

| Directory | Contents |
|-----------|----------|
| `kernel/` | the kernel (Rust + `asm/entry.s`) |
| `boot/` | the boot block |
| `user/` | the `rt` runtime library and every program (`src/bin`) |
| `libc/` | the C library: `include/` headers, `src/` sources |
| `lib/azcc` | the C compiler (preprocessor, parser, code generator) |
| `lib/azas` | the assembler |
| `lib/azld` | the linker and archive support |
| `lib/azfs` | the on-disk file system format |
| `lib/azsys` | the system call ABI shared by kernel and programs |
| `lib/azrt`, `lib/rtasm` | run-time support replacing compiler_builtins |
| `lib/azbuild` | build-script helpers |
| `root/` | files and device nodes for the root file system |
| `man/` | hand-written manual pages (others are generated from the sources) |
| `tools/` | host tools: disk builder, host front ends, test drivers |
| `toolchain/` | the LLVM M68k patches and build script |
| `tests/` | code generator, C compiler and OS tests |

The compilers, assembler and linker are `no_std` libraries, so the same code
runs on the build host (to build the system) and on AZ/UX (as `cc`, `as`
and `ld`).

## Using the system

Log in as `root` or `user`, then `man intro` is a good start; `help` lists
the shell's built-in commands and `man -l` every manual page. Example
programs are in `/usr/src/examples` and the C library sources in
`/usr/src/libc`:

```
cd /tmp
cc -o primes /usr/src/examples/primes.c
./primes 100
edit hello.c
```

The Rust-subset compiler that was originally planned is not included.
