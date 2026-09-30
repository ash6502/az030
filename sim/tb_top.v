// Bus-functional testbench: plays the role of a 68030 and pokes the SoC.
// Run:  make sim     (needs Icarus Verilog)
`timescale 1ns/1ps
module tb;
    reg clk50 = 0;
    always #10 clk50 = ~clk50;           // 50 MHz

    reg         rst_btn_n = 0;
    reg  [31:0] a = 0, dv = 0;
    reg         doe = 0, as_n = 1, ds_n = 1, rw = 1;
    reg  [1:0]  siz = 0;
    wire [31:0] d = doe ? dv : 32'bz;

    wire [1:0]  dsack_n;
    wire        berr_n, cpu_clk, cpu_reset_n, cpu_halt_n, txd;
    wire [2:0]  ipl_n;
    wire        avec_n, sterm_n, ciin_n, br_n, bgack_n;
    wire [3:0]  leds;

    top #(.BAUD(5_000_000)) dut (           // fast UART for simulation (DIV = 10)
        .clk50(clk50), .rst_btn_n(rst_btn_n),
        .cpu_clk(cpu_clk), .cpu_reset_n(cpu_reset_n), .cpu_halt_n(cpu_halt_n),
        .cpu_a(a), .cpu_d(d), .cpu_as_n(as_n), .cpu_ds_n(ds_n), .cpu_rw(rw),
        .cpu_siz(siz), .cpu_dsack_n(dsack_n), .cpu_berr_n(berr_n),
        .cpu_ipl_n(ipl_n), .cpu_avec_n(avec_n), .cpu_sterm_n(sterm_n),
        .cpu_ciin_n(ciin_n), .cpu_br_n(br_n), .cpu_bgack_n(bgack_n),
        .uart_txd(txd), .uart_rxd(1'b1), .leds(leds));

    integer     errors = 0;
    reg [31:0]  rd;
    reg         got_berr;

    // One 68030-style bus cycle
    task cycle(input [31:0] addr, input is_read, input [1:0] size, input [31:0] wdat);
        integer n;
        begin
            a = addr; rw = is_read; siz = size; got_berr = 0;
            @(posedge clk50); #1;
            as_n = 0;
            if (!is_read) begin dv = wdat; doe = 1; end
            #1 ds_n = 0;
            n = 0;
            while (dsack_n == 2'b11 && berr_n == 1'b1 && n < 2000) begin
                @(posedge clk50); n = n + 1;
            end
            #1;
            rd = d;
            got_berr = (berr_n == 1'b0);
            as_n = 1; ds_n = 1; doe = 0;
            repeat (4) @(posedge clk50);      // let the bridge return to idle
        end
    endtask

    task check(input [255:0] name, input [31:0] got, input [31:0] exp);
        begin
            if (got !== exp) begin
                $display("FAIL %0s: got %h expected %h", name, got, exp);
                errors = errors + 1;
            end else $display("ok   %0s", name);
        end
    endtask

    // Very small UART receiver (200 ns bit time)
    reg [7:0] uart_rx = 0;
    reg       uart_got = 0;
    integer   i;
    always @(negedge txd) if (cpu_reset_n) begin
        #300;
        for (i = 0; i < 8; i = i + 1) begin uart_rx[i] = txd; #200; end
        uart_got = 1;
    end

    initial begin
        $dumpfile("sim/tb.vcd");
        $dumpvars(0, tb);
        #100 rst_btn_n = 1;
        wait (cpu_reset_n === 1'b1);
        repeat (10) @(posedge clk50);

        // Reset vectors visible at 0x0 through the ROM overlay
        cycle(32'h0000_0000, 1, 2'b00, 0);        check("overlay: SSP vector", rd, 32'h0000_8000);
        cycle(32'hFFF0_0004, 1, 2'b00, 0);        check("ROM alias: PC vector", rd, 32'hFFF0_0400);

        // Drop the overlay
        cycle(32'hFE00_1000, 1, 2'b00, 0);        check("BOOT reads overlay=1", rd, 32'd1);
        cycle(32'hFE00_1000, 0, 2'b00, 32'd0);
        cycle(32'hFE00_1000, 1, 2'b00, 0);        check("BOOT reads overlay=0", rd, 32'd0);
        cycle(32'hFE00_1004, 1, 2'b00, 0);        check("SYS ID", rd, 32'h0303_0001);

        // RAM long / byte / word writes
        cycle(32'h0000_0100, 0, 2'b00, 32'hDEAD_BEEF);
        cycle(32'h0000_0100, 1, 2'b00, 0);        check("RAM long r/w", rd, 32'hDEAD_BEEF);
        cycle(32'h0000_0101, 0, 2'b01, 32'h00AA_0000);
        cycle(32'h0000_0100, 1, 2'b00, 0);        check("RAM byte write A=1", rd, 32'hDEAA_BEEF);
        cycle(32'h0000_0102, 0, 2'b10, 32'h0000_1234);
        cycle(32'h0000_0100, 1, 2'b00, 0);        check("RAM word write A=2", rd, 32'hDEAA_1234);

        // Unmapped address -> BERR, and the bus must recover afterwards
        cycle(32'h5000_0000, 1, 2'b00, 0);        check("unmapped -> BERR", {31'd0, got_berr}, 32'd1);
        cycle(32'h0000_0100, 1, 2'b00, 0);        check("recovered after BERR", rd, 32'hDEAA_1234);

        // UART transmit
        cycle(32'hFE00_0000, 0, 2'b00, 32'h0000_0041);
        #4000;
        check("UART sent 'A'", {31'd0, uart_got}, 32'd1);
        check("UART byte",     {24'd0, uart_rx}, 32'h41);
        cycle(32'hFE00_0004, 1, 2'b00, 0);        check("UART tx_ready", rd & 32'h1, 32'd1);

        if (errors == 0) $display("ALL TESTS PASSED"); else $display("%0d TEST(S) FAILED", errors);
        $finish;
    end

    initial begin #5_000_000; $display("TIMEOUT"); $finish; end
endmodule
