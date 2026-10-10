# Toolchain notes

The OS is written in Rust (plus 68k assembly) and cross-compiled with the stock
`rustc`, but two pieces are local:

* **rust-src** (downloaded by `make toolchain`) so `core` and `alloc` can be built for
  `m68k-unknown-none-elf` with `-Zbuild-std`.
* **A patched LLVM `llc`** (`build-llvm.sh`). LLVM's M68k back end is experimental and
  miscompiles ordinary Rust code, and rustc's LLVM cannot be patched in place. So rustc
  emits LLVM bitcode (`-C linker-plugin-lto`, see `os/.cargo/config.toml`) and our linker,
  azld, compiles it with the patched `llc` (cached by content hash).

Rust's `compiler_builtins` is always compiled by rustc's own (unpatched) LLVM, so the
routines it would provide (`memcpy`, 64/128-bit integer helpers, soft float) are
replaced by `lib/rtasm/*.s` (assembly; floats use the 68882) and `lib/azrt` (Rust,
compiled by the patched llc). azld lets definitions in plain objects override those in
archives, so the replacements win without touching rustc.

## LLVM M68k fixes (`llvm-m68k-fixes.patch`, against LLVM 23.1.1)

Each was found with the differential test in `os/tests/cgtest` (`make cgtest`), which
runs the same Rust code on the host and on the emulated 68030 and compares the output.

1. **Extending loads folded into memory-to-memory moves** (`isSafeStoreLoad`).
   `*u32_ptr = *u8_ptr as u32` became `move.l (src),(dst)`, copying 4 bytes.
2. **Index register dropped from frame-index addresses** (`hasIndexReg`, `SelectARID`,
   `SelectARI`). `buf[i] = x` for a stack array stored to `(d8,sp)` ignoring `i`.
3. **Register copies clobbering the condition codes** (`copyPhysReg`). A `move.l` into a
   data register sets N/Z; copies placed between a compare and its branch flipped the
   branch. CCR is now saved around such copies when it is live.
4. **fastcc struct returns popping a stack slot that was never pushed**
   (`LowerCall`/`LowerFormalArguments`). The callee overwrote the caller's `(sp)`.
5. **Sign/zero-extension pseudos allowed address-register destinations**
   (`MOV[SZ]Xd32d16`). Their expansion ANDs/EXTs the destination, which the encoder
   then emitted against the *data* register with the same number.
6. **32-bit `divs.l`/`divu.l` encoded with Dr = d0** (`MxDiMuOp_DD_Long`). The CPU
   wrote the remainder into d0, unknown to the register allocator. Dr now equals Dq.

`tools/flagcheck.py` (CCR hazards in a disassembly) and `llc -verify-machineinstrs`
were useful for finding them; azld honours `AZLD_OPT` and `AZLD_LLC_ARGS` (e.g.
`-opt-bisect-limit=N`) for debugging. llc is run with `-code-model=large` (absolute
addressing; LLVM's default assumes code and data within ±32 KB) and
`-function-sections` so unused code can be garbage collected.
