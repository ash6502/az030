.PHONY: all fw sim clean
all: fw

fw:
	$(MAKE) -C fw

sim:
	iverilog -g2005 -o sim/tb.vvp sim/tb_top.v rtl/*.v
	vvp sim/tb.vvp

clean:
	$(MAKE) -C fw clean || true
	rm -f sim/tb.vvp sim/tb.vcd
