# Boot ROM build. Needs vasm (http://sun.hasenbraten.de/vasm/), built with:
#   make CPU=m68k SYNTAX=mot
VASM    ?= vasmm68k_mot
VFLAGS  := -m68030 -m68882 -quiet

all: boot.bin examples

boot.bin: boot.s
	$(VASM) $(VFLAGS) -Fbin -o $@ $<

# Same image as a Motorola S-record file, for EPROM programmers.
boot.s37: boot.s
	$(VASM) $(VFLAGS) -Fsrec -s37 -o $@ $<

examples: examples/hello.bin

examples/%.bin: examples/%.s
	$(VASM) $(VFLAGS) -Fbin -o $@ $<

clean:
	rm -f boot.s37 examples/*.bin

.PHONY: all examples clean
