# az030 OS build.
#
#   make toolchain   download rust-src and build the patched LLVM llc (once, ~1 hour)
#   make             build everything and the bootable disk image build/disk.img
#   make run         boot it in the emulator
#   make cgtest      check the m68k code generator against the host
#
# See README.md.

OS      := $(abspath .)
ROOT    := $(abspath ..)
TC      := $(OS)/.toolchain
BUILD   := $(OS)/build
TARGET  := m68k-unknown-none-elf
RUST_VERSION := $(shell rustc --version | cut -d' ' -f2)
LLVM_VERSION := 23.1.1

EMU     := $(ROOT)/emu/target/release/az030-emu
TOOLS   := $(OS)/target/release
AZAS    := $(TOOLS)/azas
MKDISK  := $(TOOLS)/azmkdisk

# Cross-compiling Rust needs the standard library sources and nightly-only flags.
export RUSTC_BOOTSTRAP := 1
export __CARGO_TESTS_ONLY_SRC_ROOT := $(TC)/rust-src/library
XCARGO  := cargo build --release --target $(TARGET) \
           -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem
XOUT    := $(OS)/target/$(TARGET)/release

.PHONY: all disk toolchain tools emu cgtest kernel boot user root clean run $(BUILD)/disk.img

all: disk

disk: $(BUILD)/disk.img

# ---- toolchain ---------------------------------------------------------------

toolchain: $(TC)/rust-src/library $(TC)/llvm/bin/llc

$(TC)/rust-src/library:
	mkdir -p $(TC)/dl
	cd $(TC)/dl && curl -sSfLO https://static.rust-lang.org/dist/rust-src-$(RUST_VERSION).tar.xz
	cd $(TC)/dl && curl -sSfLO https://static.rust-lang.org/dist/rust-src-$(RUST_VERSION).tar.xz.sha256
	cd $(TC)/dl && shasum -a 256 -c rust-src-$(RUST_VERSION).tar.xz.sha256
	mkdir -p $(TC)/extract
	tar -xJf $(TC)/dl/rust-src-$(RUST_VERSION).tar.xz -C $(TC)/extract
	ln -sfn $(TC)/extract/rust-src-$(RUST_VERSION)/rust-src/lib/rustlib/src/rust $(TC)/rust-src

$(TC)/llvm/bin/llc:
	sh toolchain/build-llvm.sh $(LLVM_VERSION) $(TC)

# ---- host tools ----------------------------------------------------------------

tools:
	cargo build --release -p az-tools

emu:
	cargo build --release --manifest-path $(ROOT)/emu/Cargo.toml

# ---- code generator self-test --------------------------------------------------

cgtest: tools emu
	$(XCARGO) -p cgtest
	mkdir -p $(BUILD)/cgtest
	$(EMU) mkdisk $(BUILD)/cgtest/disk.img 1M --boot $(XOUT)/cgtest --load 0x10000 \
	    --entry 0x$$(grep ' _start$$' $$(ls -t $(XOUT)/build/cgtest-*/out/cgtest.map | head -1) | cut -d' ' -f1) >/dev/null
	printf '[machine]\nram_size = "16M"\nthrottle = false\n[rom]\nfile = "%s"\n[scsi]\nenabled = true\n[[scsi.disk]]\nid = 0\nimage = "%s"\n' \
	    $(ROOT)/rom/boot.bin $(BUILD)/cgtest/disk.img > $(BUILD)/cgtest/emu.toml
	$(EMU) -c $(BUILD)/cgtest/emu.toml --max-cycles 4000000000 < /dev/null \
	    | tr -d '\r' | sed -n '/^=== BEGIN ===$$/,/^=== END ===$$/p' | sed '1d;$$d' > $(BUILD)/cgtest/m68k.txt
	$(TOOLS)/cgtest-host > $(BUILD)/cgtest/host.txt
	diff -u $(BUILD)/cgtest/host.txt $(BUILD)/cgtest/m68k.txt && echo "cgtest: PASS ($$(wc -l < $(BUILD)/cgtest/host.txt) lines identical)"

# ---- kernel and bootloader ---------------------------------------------------------

KERNEL_LOAD := 0x10000

kernel: tools
	$(XCARGO) -p kernel
	mkdir -p $(BUILD)
	cp $(XOUT)/kernel $(BUILD)/kernel.bin
	cp $$(ls -t $(XOUT)/build/kernel-*/out/kernel.map | head -1) $(BUILD)/kernel.map

boot: tools
	mkdir -p $(BUILD)
	$(AZAS) -f bin -o $(BUILD)/boot.bin boot/boot.s

# ---- disk image -------------------------------------------------------------------------

DISK_SIZE ?= 64M
CMDLINE   ?=

$(BUILD)/disk.img: kernel boot root
	$(MKDISK) -o $@ --size $(DISK_SIZE) --root $(BUILD)/root --manifest root/manifest \
	    --boot $(BUILD)/boot.bin --kernel $(BUILD)/kernel.bin --load $(KERNEL_LOAD) \
	    --entry 0x$$(grep ' _start$$' $(BUILD)/kernel.map | cut -d' ' -f1) --cmdline "$(CMDLINE)"

# The root file system is assembled in build/root from os/root (static files) and
# the user programs.
SBIN_PROGS := init halt mount umount
# extra names for programs that behave differently depending on argv[0]
ALIASES := chown:chgrp readlink:realpath grep:egrep grep:fgrep test:[ od:hexdump \
           more:less halt:reboot halt:poweroff id:groups

user: tools
	$(XCARGO) -p user

root: user
	rm -rf $(BUILD)/root
	mkdir -p $(BUILD)/root/bin $(BUILD)/root/sbin
	cp -R root/files/. $(BUILD)/root/
	for src in user/src/bin/*.rs user/src/bin/*/main.rs; do \
	    p=$$(basename $$src .rs); [ $$p = main ] && p=$$(basename $$(dirname $$src)); \
	    case " $(SBIN_PROGS) " in *" $$p "*) d=sbin;; *) d=bin;; esac; \
	    install -m 755 $(XOUT)/$$p $(BUILD)/root/$$d/$$p; \
	done
	for a in $(ALIASES); do \
	    src=$${a%%:*}; dst=$${a#*:}; \
	    d=bin; [ -f $(BUILD)/root/sbin/$$src ] && d=sbin; \
	    ln -sf $$src "$(BUILD)/root/$$d/$$dst"; \
	done

run: $(BUILD)/disk.img emu
	sed -e 's|@DISK@|$(BUILD)/disk.img|' -e 's|@ROM@|$(ROOT)/rom/boot.bin|' az030.toml.in > $(BUILD)/az030.toml
	$(EMU) -c $(BUILD)/az030.toml

clean:
	rm -rf $(BUILD)
	cargo clean
