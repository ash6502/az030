// Boot ROM (block RAM initialised from a hex file, one 32-bit word per line).
module rom #(
    parameter AW   = 12,                 // words -> 4096 x 4 = 16 KB
    parameter INIT = "fw/boot.hex"
)(
    input  wire          clk,
    input  wire          rst_n,
    input  wire          req,
    input  wire          sel,
    input  wire [AW-1:0] a,              // word address
    output reg  [31:0]   rdata,
    output reg           ack
);
    reg [31:0] mem [0:(1<<AW)-1];
    initial $readmemh(INIT, mem);

    wire go = req & sel & ~ack;
    always @(posedge clk) begin
        if (!rst_n) ack <= 1'b0; else ack <= go;
        rdata <= mem[a];
    end
endmodule
