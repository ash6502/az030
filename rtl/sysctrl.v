// System control registers (32-bit accesses only)
//   +0x0  BOOT : read bit0 = ROM overlay at 0x0 active; ANY write clears it
//   +0x4  ID   : read-only 0x03030001
//   +0x8  LEDS : read/write [3:0]
module sysctrl (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        req,
    input  wire        sel,
    input  wire        we,
    input  wire [1:0]  reg_a,            // addr[3:2]
    input  wire [31:0] wdata,
    output reg  [31:0] rdata,
    output reg         ack,
    output reg         overlay,
    output reg  [3:0]  leds
);
    wire go = req & sel & ~ack;
    always @(posedge clk) begin
        if (!rst_n) begin
            ack     <= 1'b0;
            overlay <= 1'b1;             // ROM appears at 0x0 after reset
            leds    <= 4'd0;
        end else begin
            ack <= go;
            if (go && we) begin
                case (reg_a)
                    2'd0: overlay <= 1'b0;
                    2'd2: leds    <= wdata[3:0];
                    default: ;
                endcase
            end
        end
        case (reg_a)
            2'd0:    rdata <= {31'd0, overlay};
            2'd1:    rdata <= 32'h0303_0001;
            2'd2:    rdata <= {28'd0, leds};
            default: rdata <= 32'd0;
        endcase
    end
endmodule
