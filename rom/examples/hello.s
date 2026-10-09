; Example program for the az030 boot ROM.
; Build a bootable disk image with:
;   make -C rom examples
;   cargo run --release --manifest-path emu/Cargo.toml -- mkdisk emu/disks/hello.img 1M \
;       --boot rom/examples/hello.bin --load 0x10000
;
; Uses the ROM's TRAP #15 calls (see boot.s) and the FPU.

        org     $10000

start:
        lea     msg,a0
        moveq   #3,d0                   ; puts
        trap    #15

        ; compute sqrt(2) * 1000000 with the FPU and print it
        fmove.l #2,fp0
        fsqrt.x fp0
        fmul.l  #1000000,fp0
        fmove.l fp0,d2
        bsr     putdec
        lea     crlf,a0
        moveq   #3,d0
        trap    #15

        lea     prompt,a0
        moveq   #3,d0
        trap    #15
.echo:  moveq   #2,d0                   ; getc -> d1
        trap    #15
        cmp.b   #'q',d1
        beq.s   .done
        moveq   #1,d0                   ; putc d1
        trap    #15
        bra.s   .echo
.done:  moveq   #0,d0                   ; exit to monitor
        trap    #15

; print d2 as unsigned decimal via TRAP #15
putdec: moveq   #0,d3
.div:   divul.l #10,d1:d2
        move.w  d1,-(sp)
        addq.l  #1,d3
        tst.l   d2
        bne.s   .div
.out:   move.w  (sp)+,d1
        add.b   #'0',d1
        moveq   #1,d0
        trap    #15
        subq.l  #1,d3
        bne.s   .out
        rts

msg:    dc.b    "Hello from a program booted off SCSI!",13,10
        dc.b    "sqrt(2) * 1e6 = ",0
crlf:   dc.b    13,10,0
prompt: dc.b    "Echoing input, press q to quit: ",0
