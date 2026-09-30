// -----------------------------------------------------------------------------
// bus_bridge.v  --  68030 asynchronous bus  <->  simple internal bus
//
// This is the ONLY CPU-specific module. Swap it out to change CPUs.
//
// Internal bus protocol (all synchronous to clk, which should be >= 2x CPU clk):
//   - bridge raises `req` and holds addr/we/be/wdata stable
//   - the selected slave answers with a ONE-cycle `ack` pulse, rdata valid then
//   - bridge drops `req` the cycle after it sees `ack`
//   - no ack within 256 clk cycles  ->  BERR* to the CPU (unmapped address)
//
// be[3] = D[31:24] ... be[0] = D[7:0]   (68030 big-endian byte lanes)
// All ports are presented to the CPU as 32-bit (DSACK1*=DSACK0*=0).
// -----------------------------------------------------------------------------
module bus_bridge (
    input  wire        clk,
    input  wire        rst_n,

    // 68030 side
    input  wire [31:0] cpu_a,
    input  wire [31:0] cpu_d_in,
    output reg  [31:0] cpu_d_out,
    output reg         cpu_d_oe,
    input  wire        cpu_as_n,
    input  wire        cpu_ds_n,
    input  wire        cpu_rw,          // 1 = read, 0 = write
    input  wire [1:0]  cpu_siz,
    output reg  [1:0]  cpu_dsack_n,
    output reg         cpu_berr_n,

    // internal bus (master side)
    output reg         req,
    output reg         we,
    output reg  [31:0] addr,
    output reg  [31:0] wdata,
    output reg  [3:0]  be,
    input  wire [31:0] rdata,
    input  wire        ack
);

    // Synchronise the async strobes (active-low -> active-high)
    reg [1:0] as_sr, ds_sr;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            as_sr <= 2'b00;
            ds_sr <= 2'b00;
        end else begin
            as_sr <= {as_sr[0], ~cpu_as_n};
            ds_sr <= {ds_sr[0], ~cpu_ds_n};
        end
    end
    wire as_s = as_sr[1];
    wire ds_s = ds_sr[1];

    // Byte enables from SIZ[1:0] and A[1:0] for a 32-bit port
    reg [3:0] be_c;
    always @* begin
        case (cpu_siz)
            2'b01: case (cpu_a[1:0])                 // byte
                       2'b00: be_c = 4'b1000;
                       2'b01: be_c = 4'b0100;
                       2'b10: be_c = 4'b0010;
                       2'b11: be_c = 4'b0001;
                   endcase
            2'b10: case (cpu_a[1:0])                 // word
                       2'b00: be_c = 4'b1100;
                       2'b01: be_c = 4'b0110;
                       2'b10: be_c = 4'b0011;
                       2'b11: be_c = 4'b0001;
                   endcase
            2'b11: case (cpu_a[1:0])                 // 3 bytes
                       2'b00: be_c = 4'b1110;
                       2'b01: be_c = 4'b0111;
                       2'b10: be_c = 4'b0011;
                       2'b11: be_c = 4'b0001;
                   endcase
            default: case (cpu_a[1:0])               // long
                       2'b00: be_c = 4'b1111;
                       2'b01: be_c = 4'b0111;
                       2'b10: be_c = 4'b0011;
                       2'b11: be_c = 4'b0001;
                   endcase
        endcase
    end

    localparam S_IDLE = 2'd0, S_WAIT = 2'd1, S_DONE = 2'd2;
    reg [1:0] st;
    reg [7:0] tmo;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            st          <= S_IDLE;
            req         <= 1'b0;
            we          <= 1'b0;
            addr        <= 32'd0;
            wdata       <= 32'd0;
            be          <= 4'd0;
            tmo         <= 8'd0;
            cpu_d_out   <= 32'd0;
            cpu_d_oe    <= 1'b0;
            cpu_dsack_n <= 2'b11;
            cpu_berr_n  <= 1'b1;
        end else begin
            case (st)
            S_IDLE: begin
                // Reads can start on AS*; writes wait for DS* (data valid)
                if (as_s && (cpu_rw || ds_s)) begin
                    addr  <= cpu_a;
                    we    <= ~cpu_rw;
                    be    <= be_c;
                    wdata <= cpu_d_in;
                    req   <= 1'b1;
                    tmo   <= 8'd0;
                    st    <= S_WAIT;
                end
            end
            S_WAIT: begin
                tmo <= tmo + 8'd1;
                if (!as_s) begin                 // CPU aborted the cycle
                    req <= 1'b0;
                    st  <= S_IDLE;
                end else if (ack) begin
                    req         <= 1'b0;
                    cpu_d_out   <= rdata;
                    cpu_d_oe    <= cpu_rw;
                    cpu_dsack_n <= 2'b00;        // 32-bit port termination
                    st          <= S_DONE;
                end else if (&tmo) begin         // nobody answered
                    req        <= 1'b0;
                    cpu_berr_n <= 1'b0;
                    st         <= S_DONE;
                end
            end
            S_DONE: begin
                if (!as_s) begin
                    cpu_dsack_n <= 2'b11;
                    cpu_berr_n  <= 1'b1;
                    cpu_d_oe    <= 1'b0;
                    st          <= S_IDLE;
                end
            end
            default: st <= S_IDLE;
            endcase
        end
    end
endmodule
