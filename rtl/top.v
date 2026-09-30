// -----------------------------------------------------------------------------
// top.v -- minimal 68030 SoC glue
//
// Memory map (see README.md):
//   0x0000_0000  RAM (bring-up: 32 KB BRAM; later SDRAM)   [ROM overlay at reset]
//   0xFD00_0000  VRAM      (reserved)
//   0xFE00_0000  UART
//   0xFE00_1000  SYSCTRL
//   0xFE00_2000  SCSI      (reserved)
//   0xFE00_3000  VIDEO regs(reserved)
//   0xFFF0_0000  Boot ROM (16 KB, mirrored across 1 MB)
// -----------------------------------------------------------------------------
module top #(
    parameter CLK_HZ = 50_000_000,
    parameter BAUD   = 115_200,
    parameter ROM_INIT = "fw/boot.hex"
)(
    input  wire        clk50,            // FPGA oscillator; CPU clock = clk50 / 2
    input  wire        rst_btn_n,

    // ---- 68030 pins (use level shifters: 68030 is 5 V, FPGA is 3.3 V or less)
    output wire        cpu_clk,
    output wire        cpu_reset_n,
    output wire        cpu_halt_n,
    input  wire [31:0] cpu_a,
    inout  wire [31:0] cpu_d,
    input  wire        cpu_as_n,
    input  wire        cpu_ds_n,
    input  wire        cpu_rw,
    input  wire [1:0]  cpu_siz,
    output wire [1:0]  cpu_dsack_n,
    output wire        cpu_berr_n,
    // tie-offs for inputs we don't use yet
    output wire [2:0]  cpu_ipl_n,        // no interrupts
    output wire        cpu_avec_n,
    output wire        cpu_sterm_n,      // no synchronous/burst termination
    output wire        cpu_ciin_n,       // TODO: assert for I/O regions
    output wire        cpu_br_n,         // no other bus masters
    output wire        cpu_bgack_n,

    // ---- board peripherals
    output wire        uart_txd,
    input  wire        uart_rxd,
    output wire [3:0]  leds
);

    // ---- CPU clock and power-on reset --------------------------------------
    reg cpu_clk_r = 1'b0;
    always @(posedge clk50) cpu_clk_r <= ~cpu_clk_r;
    assign cpu_clk = cpu_clk_r;

    reg [15:0] rst_cnt = 16'd0;          // ~1.3 ms at 50 MHz (68030 wants >= 520 CPU clocks)
    reg        rst_n_r = 1'b0;
    always @(posedge clk50) begin
        if (!rst_btn_n) begin
            rst_cnt <= 16'd0;
            rst_n_r <= 1'b0;
        end else if (rst_cnt != 16'hFFFF) begin
            rst_cnt <= rst_cnt + 16'd1;
        end else begin
            rst_n_r <= 1'b1;
        end
    end
    wire rst_n = rst_n_r;
    assign cpu_reset_n = rst_n;
    assign cpu_halt_n  = rst_n;

    assign cpu_ipl_n   = 3'b111;
    assign cpu_avec_n  = 1'b1;
    assign cpu_sterm_n = 1'b1;
    assign cpu_ciin_n  = 1'b1;
    assign cpu_br_n    = 1'b1;
    assign cpu_bgack_n = 1'b1;

    // ---- CPU bridge ----------------------------------------------------------
    wire [31:0] cpu_d_out;
    wire        cpu_d_oe;
    assign cpu_d = cpu_d_oe ? cpu_d_out : 32'bz;

    wire        req, we, ack;
    wire [31:0] addr, wdata, rdata;
    wire [3:0]  be;

    bus_bridge u_bridge (
        .clk(clk50), .rst_n(rst_n),
        .cpu_a(cpu_a), .cpu_d_in(cpu_d), .cpu_d_out(cpu_d_out), .cpu_d_oe(cpu_d_oe),
        .cpu_as_n(cpu_as_n), .cpu_ds_n(cpu_ds_n), .cpu_rw(cpu_rw), .cpu_siz(cpu_siz),
        .cpu_dsack_n(cpu_dsack_n), .cpu_berr_n(cpu_berr_n),
        .req(req), .we(we), .addr(addr), .wdata(wdata), .be(be),
        .rdata(rdata), .ack(ack)
    );

    // ---- address decode ------------------------------------------------------
    wire overlay;
    wire sel_rom  = (addr[31:20] == 12'hFFF) || (overlay && addr[31:20] == 12'h000);
    wire sel_ram  = !overlay && (addr[31:15] == 17'd0);            // 32 KB for now
    wire sel_io   = (addr[31:16] == 16'hFE00);
    wire sel_uart = sel_io && (addr[15:12] == 4'h0);
    wire sel_sys  = sel_io && (addr[15:12] == 4'h1);
    // sel_scsi = sel_io && addr[15:12]==4'h2;   -- TODO
    // sel_vid  = sel_io && addr[15:12]==4'h3;   -- TODO

    // ---- slaves --------------------------------------------------------------
    wire [31:0] rom_rd, ram_rd, uart_rd, sys_rd;
    wire        rom_ack, ram_ack, uart_ack, sys_ack;

    rom #(.AW(12), .INIT(ROM_INIT)) u_rom (
        .clk(clk50), .rst_n(rst_n), .req(req), .sel(sel_rom),
        .a(addr[13:2]), .rdata(rom_rd), .ack(rom_ack));

    ram_bram #(.AW(13)) u_ram (
        .clk(clk50), .rst_n(rst_n), .req(req), .sel(sel_ram), .we(we), .be(be),
        .a(addr[14:2]), .wdata(wdata), .rdata(ram_rd), .ack(ram_ack));

    uart #(.CLK_HZ(CLK_HZ), .BAUD(BAUD)) u_uart (
        .clk(clk50), .rst_n(rst_n), .req(req), .sel(sel_uart), .we(we),
        .reg_a(addr[2]), .wdata(wdata), .rdata(uart_rd), .ack(uart_ack),
        .rxd(uart_rxd), .txd(uart_txd));

    sysctrl u_sys (
        .clk(clk50), .rst_n(rst_n), .req(req), .sel(sel_sys), .we(we),
        .reg_a(addr[3:2]), .wdata(wdata), .rdata(sys_rd), .ack(sys_ack),
        .overlay(overlay), .leds(leds));

    // ---- read-data / ack mux -------------------------------------------------
    assign ack   = rom_ack | ram_ack | uart_ack | sys_ack;
    assign rdata = rom_ack  ? rom_rd  :
                   ram_ack  ? ram_rd  :
                   uart_ack ? uart_rd :
                   sys_ack  ? sys_rd  : 32'd0;
endmodule
