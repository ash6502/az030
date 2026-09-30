CROSS   ?= m68k-elf-
AS      := $(CROSS)as
LD      := $(CROSS)ld
OBJCOPY := $(CROSS)objcopy

all: boot.hex

boot.o: boot.S
	$(AS) -m68030 -o $@ $<

boot.elf: boot.o link.ld
	$(LD) -T link.ld -o $@ boot.o

boot.bin: boot.elf
	$(OBJCOPY) -O binary $< $@

boot.hex: boot.bin bin2hex.py
	python3 bin2hex.py $< $@

clean:
	rm -f *.o *.elf *.bin *.hex
.PHONY: all clean
