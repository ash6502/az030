;=============================================================================
; az030 OS bootloader
;
; The boot ROM loads this from LBA 1 to $8000 (see the AZ30BOOT block) and
; calls it with d0 = SCSI ID and d1 = RAM size. It reads the AZOS fields of the
; boot block, loads the kernel with the ROM's disk-read call, fills in a
; boot-info block and jumps to the kernel:
;
;   a0 = boot info, d0 = SCSI ID, d1 = RAM size, interrupts masked.
;
; Boot block (LBA 0) fields used here, big-endian:
;   $20  "AZOS"
;   $24  kernel LBA          $28  kernel length in blocks
;   $2C  kernel load address $30  kernel entry
;   $34  root fs LBA         $38  root fs length in blocks
;   $3C  kernel length in bytes
;   $40  kernel command line (NUL-terminated, up to 128 bytes)
;
; Assemble: azas -f bin -o boot.bin boot.s
;=============================================================================

LOAD_ADDR       equ     $8000
CHUNK           equ     128                     ; blocks per ROM read (64 KB)

BI_MAGIC        equ     $424F4F54               ; "BOOT"

        org     LOAD_ADDR

start:
        move.w  #$2700,sr
        move.l  d0,boot_id
        move.l  d1,ram_size
        lea     msg_hello(pc),a0
        bsr     puts

        ; read the boot block
        move.l  boot_id,d1
        moveq   #0,d2
        moveq   #1,d3
        lea     bootblk,a0
        moveq   #5,d0
        trap    #15
        tst.l   d0
        bne     ioerr
        cmp.l   #'AZOS',bootblk+$20
        bne     nokernel

        ; check the kernel fits in RAM
        move.l  bootblk+$2C,d0
        move.l  bootblk+$28,d1
        moveq   #9,d2
        lsl.l   d2,d1
        add.l   d1,d0
        cmp.l   ram_size,d0
        bhi     toobig

        lea     msg_loading(pc),a0
        bsr     puts

        ; load the kernel in chunks, printing a dot for each
        move.l  bootblk+$24,d2                  ; LBA
        move.l  bootblk+$28,d4                  ; blocks left
        move.l  bootblk+$2C,a0                  ; destination
.chunk: tst.l   d4
        beq.s   .loaded
        move.l  d4,d3
        cmp.l   #CHUNK,d3
        bls.s   .n
        move.l  #CHUNK,d3
.n:     move.l  boot_id,d1
        moveq   #5,d0
        trap    #15                             ; read d3 blocks at d2 into (a0)
        tst.l   d0
        bne     ioerr
        add.l   d3,d2
        sub.l   d3,d4
        move.l  d3,d5
        moveq   #9,d6
        lsl.l   d6,d5
        add.l   d5,a0
        moveq   #'.',d1
        moveq   #1,d0
        trap    #15
        bra.s   .chunk
.loaded:
        lea     msg_ok(pc),a0
        bsr     puts

        ; boot info
        lea     bootinfo,a1
        move.l  #BI_MAGIC,(a1)+
        move.l  ram_size,(a1)+
        move.l  boot_id,(a1)+
        move.l  bootblk+$34,(a1)+
        move.l  bootblk+$38,(a1)+
        move.l  bootblk+$2C,(a1)+
        move.l  bootblk+$3C,(a1)+
        lea     bootblk+$40,a2
        move.w  #128-1,d0
.cmd:   move.b  (a2)+,(a1)+
        dbra    d0,.cmd
        clr.b   -1(a1)                          ; always terminated

        move.l  bootblk+$30,a2
        lea     bootinfo,a0
        move.l  boot_id,d0
        move.l  ram_size,d1
        jmp     (a2)

ioerr:  lea     msg_ioerr(pc),a0
        bra.s   fail
nokernel:
        lea     msg_nokernel(pc),a0
        bra.s   fail
toobig: lea     msg_toobig(pc),a0
fail:   bsr     puts
        moveq   #0,d0                           ; back to the ROM monitor
        trap    #15

; puts: a0 = NUL-terminated string (via the ROM)
puts:   moveq   #3,d0
        trap    #15
        rts

msg_hello:      dc.b    13,10,"az030 bootloader",13,10,0
msg_loading:    dc.b    "Loading kernel ",0
msg_ok:         dc.b    " ok",13,10,0
msg_ioerr:      dc.b    13,10,"boot: disk read error",13,10,0
msg_nokernel:   dc.b    "boot: no AZOS kernel on this disk",13,10,0
msg_toobig:     dc.b    "boot: kernel does not fit in RAM",13,10,0
        even

boot_id:        ds.l    1
ram_size:       ds.l    1
bootinfo:       ds.b    28+128
        even
bootblk:        ds.b    512
