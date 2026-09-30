// Minimal 8N1 UART (32-bit accesses only)
//   +0x0 DATA   : write [7:0] = transmit; read [7:0] = received byte (clears rx_valid)
//   +0x4 STATUS : read bit0 = tx_ready, bit1 = rx_valid
module uart #(
    parameter CLK_HZ = 50_000_000,
    parameter BAUD   = 115_200
)(
    input  wire        clk,
    input  wire        rst_n,
    input  wire        req,
    input  wire        sel,
    input  wire        we,
    input  wire        reg_a,            // addr[2]
    input  wire [31:0] wdata,
    output reg  [31:0] rdata,
    output reg         ack,
    input  wire        rxd,
    output wire        txd
);
    localparam integer DIV = CLK_HZ / BAUD;

    wire go = req & sel & ~ack;

    // ---------------- TX ----------------
    reg [9:0]  tx_shift;
    reg [3:0]  tx_cnt;
    reg [15:0] tx_div;
    reg        tx_busy;
    assign txd = tx_busy ? tx_shift[0] : 1'b1;

    // ---------------- RX ----------------
    reg [1:0]  rx_sr;
    reg        rx_busy;
    reg [15:0] rx_div;
    reg [3:0]  rx_cnt;
    reg [7:0]  rx_sh, rx_data;
    reg        rx_valid;
    wire       rxs = rx_sr[1];

    always @(posedge clk) begin
        if (!rst_n) begin
            ack <= 1'b0;
            tx_busy <= 1'b0; tx_shift <= 10'h3FF; tx_cnt <= 4'd0; tx_div <= 16'd0;
            rx_sr <= 2'b11; rx_busy <= 1'b0; rx_div <= 16'd0; rx_cnt <= 4'd0;
            rx_sh <= 8'd0; rx_data <= 8'd0; rx_valid <= 1'b0;
        end else begin
            ack   <= go;
            rx_sr <= {rx_sr[0], rxd};

            // TX engine
            if (tx_busy) begin
                if (tx_div == DIV-1) begin
                    tx_div   <= 16'd0;
                    tx_shift <= {1'b1, tx_shift[9:1]};
                    if (tx_cnt == 4'd9) tx_busy <= 1'b0;
                    else                tx_cnt  <= tx_cnt + 4'd1;
                end else tx_div <= tx_div + 16'd1;
            end else if (go && we && !reg_a) begin
                tx_shift <= {1'b1, wdata[7:0], 1'b0};   // stop, data, start
                tx_busy  <= 1'b1;
                tx_cnt   <= 4'd0;
                tx_div   <= 16'd0;
            end

            // Clear rx_valid on data read (set below wins if a byte lands now)
            if (go && !we && !reg_a) rx_valid <= 1'b0;

            // RX engine
            if (!rx_busy) begin
                if (!rxs) begin                          // start bit edge
                    rx_busy <= 1'b1;
                    rx_div  <= DIV/2;                    // -> middle of start bit
                    rx_cnt  <= 4'd0;
                end
            end else if (rx_div != 0) begin
                rx_div <= rx_div - 16'd1;
            end else begin
                rx_div <= DIV-1;
                if (rx_cnt == 4'd0) begin
                    if (rxs) rx_busy <= 1'b0;            // glitch, abort
                    else     rx_cnt  <= 4'd1;
                end else if (rx_cnt <= 4'd8) begin
                    rx_sh  <= {rxs, rx_sh[7:1]};         // LSB first
                    rx_cnt <= rx_cnt + 4'd1;
                end else begin                           // stop bit
                    rx_data  <= rx_sh;
                    rx_valid <= 1'b1;
                    rx_busy  <= 1'b0;
                end
            end
        end
        rdata <= reg_a ? {30'd0, rx_valid, ~tx_busy} : {24'd0, rx_data};
    end
endmodule
