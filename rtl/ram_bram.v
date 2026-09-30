// Bring-up RAM in block RAM with byte enables (4 byte-wide banks).
// Replace with an SDRAM controller once the CPU boots (same internal-bus ports).
module ram_bram #(
    parameter AW = 13                    // words -> 8192 x 4 = 32 KB
)(
    input  wire          clk,
    input  wire          rst_n,
    input  wire          req,
    input  wire          sel,
    input  wire          we,
    input  wire [3:0]    be,
    input  wire [AW-1:0] a,
    input  wire [31:0]   wdata,
    output reg  [31:0]   rdata,
    output reg           ack
);
    reg [7:0] m3 [0:(1<<AW)-1];         // D[31:24]
    reg [7:0] m2 [0:(1<<AW)-1];         // D[23:16]
    reg [7:0] m1 [0:(1<<AW)-1];         // D[15:8]
    reg [7:0] m0 [0:(1<<AW)-1];         // D[7:0]

    wire go = req & sel & ~ack;
    always @(posedge clk) begin
        if (!rst_n) ack <= 1'b0; else ack <= go;
        if (go && we) begin
            if (be[3]) m3[a] <= wdata[31:24];
            if (be[2]) m2[a] <= wdata[23:16];
            if (be[1]) m1[a] <= wdata[15:8];
            if (be[0]) m0[a] <= wdata[7:0];
        end
        rdata <= {m3[a], m2[a], m1[a], m0[a]};
    end
endmodule
