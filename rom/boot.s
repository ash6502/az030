;=============================================================================
; az030 boot ROM / monitor
;
; Assemble:  vasmm68k_mot -Fbin -m68030 -m68882 -quiet -o boot.bin boot.s
;
; At reset the ROM is overlaid at address 0, so the CPU fetches the reset
; SSP/PC from the first two longs below. The reset PC points at the ROM's
; real address (0xFFF00000+), so writing SYSCTRL BOOT to drop the overlay is
; safe from the very first instruction.
;
; Boot sequence:
;   1. drop the overlay, point VBR at the ROM vector table
;   2. size RAM (probe in 32 KB steps until a bus error or aliasing)
;   3. detect the FPU (Line-F trap = none)
;   4. scan the SCSI bus (if the controller is present)
;   5. autoboot the first disk carrying an "AZ30BOOT" boot block, unless a
;      key is pressed; otherwise drop into the monitor
;
; Low RAM used by the ROM: $400-$7FF (variables, line buffer, SCSI buffer).
; Programs should load at $1000 or above. The stack sits at the top of RAM.
;
; Programs get a small system-call interface through TRAP #15 (see trap15).
;=============================================================================

ROM_BASE        equ     $FFF00000
ROM_SIZE        equ     $4000           ; 16 KB, as on the FPGA

UART_DATA       equ     $FE000000
UART_STATUS     equ     $FE000004       ; bit0 tx_ready, bit1 rx_valid
SYS_BOOT        equ     $FE001000
SYS_ID          equ     $FE001004
SYS_LEDS        equ     $FE001008

SCSI_BASE       equ     $FE002000
SCSI_CMD        equ     SCSI_BASE+$00
SCSI_STATUS     equ     SCSI_BASE+$04
SCSI_TARGET     equ     SCSI_BASE+$08
SCSI_DMA_ADDR   equ     SCSI_BASE+$0C
SCSI_DMA_LEN    equ     SCSI_BASE+$10
SCSI_XFER       equ     SCSI_BASE+$14
SCSI_CDB_LEN    equ     SCSI_BASE+$18
SCSI_ID         equ     SCSI_BASE+$1C
SCSI_CDB        equ     SCSI_BASE+$20
SCSI_ID_MAGIC   equ     $53435349       ; "SCSI"
SCSI_ERRMASK    equ     $FF0C           ; status byte | NO_TARGET | DMA_ERR

INIT_SP         equ     $8000           ; stack until RAM is sized
RAM_MAX         equ     $40000000       ; 1 GB
PROBE_STEP      equ     $8000
HOST_ID         equ     7

; --- RAM variables --------------------------------------------------------
catch_pc        equ     $400    ; nonzero: exceptions longjmp here
catch_sp        equ     $404
ram_top         equ     $408
last_addr       equ     $40C    ; for "d" with no address
last_entry      equ     $410    ; for "g" with no address
fpu_flag        equ     $414    ; byte
scsi_ok         equ     $415    ; byte
cdb             equ     $420    ; 16 bytes
linebuf         equ     $440
LINEMAX         equ     128
scsibuf         equ     $600    ; 512 bytes

AUTOBOOT_SECS   equ     3
SPIN_PER_SEC    equ     1250000 ; poll iterations per second at 25 MHz

; Arm the exception catcher: any exception resumes at \1 with SP restored.
CATCH   macro
        move.l  sp,catch_sp
        move.l  #\1,catch_pc
        endm

UNCATCH macro
        clr.l   catch_pc
        endm

; Print an inline string literal.
PRINT   macro
        lea     \1,a0
        bsr     puts
        endm

        org     ROM_BASE
rom_start:

;=============================================================================
; Vector table (VBR points here after reset)
;=============================================================================
vectors:
        dc.l    INIT_SP                 ; 0  reset SSP
        dc.l    cold_start              ; 1  reset PC
        rept    45                      ; 2..46
        dc.l    exc_entry
        endr
        dc.l    trap15                  ; 47 TRAP #15 system calls
        rept    208                     ; 48..255
        dc.l    exc_entry
        endr

;=============================================================================
; Reset
;=============================================================================
cold_start:
        move.w  #$2700,sr
        move.l  #1,SYS_BOOT             ; drop the ROM overlay: RAM at 0 now
        lea     vectors,a0
        movec   a0,vbr
        moveq   #0,d0
        movec   d0,cacr
        move.l  #1,SYS_LEDS

        ; clear the ROM's variable area ($400-$5FF)
        lea     $400,a0
        move.w  #($200/4)-1,d0
.clr:   clr.l   (a0)+
        dbra    d0,.clr

        bsr     size_ram
        move.l  d0,ram_top
        move.l  d0,sp
        move.l  #3,SYS_LEDS

        bsr     detect_fpu
        bsr     detect_scsi

        PRINT   msg_banner
        bsr     print_sysinfo

        tst.b   scsi_ok
        beq     .nodisk
        PRINT   msg_scsi_scan
        bsr     scsi_scan
        bsr     find_boot               ; d0 = bootable ID or -1
        tst.l   d0
        bmi     .nodisk
        bsr     autoboot_prompt         ; returns only if a key was pressed
.nodisk:
        move.l  #7,SYS_LEDS
        bra     monitor

;-----------------------------------------------------------------------------
; size_ram: returns d0 = bytes of contiguous RAM starting at 0.
; Stops at the first address that bus-errors, does not read back, or aliases
; onto address 0.
;-----------------------------------------------------------------------------
size_ram:
        move.l  #$A55A0FF0,d2
        move.l  d2,0                    ; alias marker at address 0
        move.l  #PROBE_STEP,a2
        CATCH   .done
.loop:  cmp.l   #RAM_MAX,a2
        bhs.s   .done
        move.l  a2,(a2)
        cmp.l   (a2),a2
        bne.s   .done
        cmp.l   0,d2
        bne.s   .done
        add.l   #PROBE_STEP,a2
        bra.s   .loop
.done:  UNCATCH
        clr.l   0
        move.l  a2,d0
        rts

detect_fpu:
        CATCH   .none
        fmove.l fpcr,d0
        UNCATCH
        st      fpu_flag
        rts
.none:  sf      fpu_flag
        rts

detect_scsi:
        CATCH   .none
        move.l  SCSI_ID,d0
        UNCATCH
        cmp.l   #SCSI_ID_MAGIC,d0
        bne.s   .none
        move.l  #2,SCSI_CMD             ; bus reset
        st      scsi_ok
        rts
.none:  sf      scsi_ok
        rts

print_sysinfo:
        PRINT   msg_cpu
        tst.b   fpu_flag
        beq.s   .nofpu
        PRINT   msg_fpu_yes
        bra.s   .ram
.nofpu: PRINT   msg_fpu_no
.ram:   PRINT   msg_ram
        move.l  ram_top,d0
        bsr     putsize
        PRINT   msg_crlf
        PRINT   msg_scsi
        tst.b   scsi_ok
        beq.s   .noscsi
        PRINT   msg_present
        rts
.noscsi:
        PRINT   msg_absent
        rts

; putsize: print d0 bytes as "N MB" or "N KB"
putsize:
        move.l  d0,-(sp)
        move.l  d0,d1
        and.l   #$FFFFF,d1
        bne.s   .kb
        moveq   #20,d1
        lsr.l   d1,d0
        bsr     putdec
        PRINT   msg_mb
        bra.s   .out
.kb:    moveq   #10,d1
        lsr.l   d1,d0
        bsr     putdec
        PRINT   msg_kb
.out:   move.l  (sp)+,d0
        rts

;-----------------------------------------------------------------------------
; autoboot_prompt: d0 = SCSI ID to boot. Counts down; any key aborts (returns).
;-----------------------------------------------------------------------------
autoboot_prompt:
        move.l  d0,d7
        PRINT   msg_autoboot1
        move.l  d7,d0
        bsr     putdec
        PRINT   msg_autoboot2
        moveq   #AUTOBOOT_SECS,d4
.sec:   move.l  d4,d0
        add.b   #'0',d0
        bsr     putc
        move.b  #' ',d0
        bsr     putc
        move.l  #SPIN_PER_SEC,d3
.spin:  move.l  UART_STATUS,d0
        btst    #1,d0
        bne.s   .key
        subq.l  #1,d3
        bne.s   .spin
        subq.l  #1,d4
        bne.s   .sec
        PRINT   msg_crlf
        move.l  d7,d0
        bra     boot_disk               ; no return on success
.key:   move.l  UART_DATA,d0            ; swallow the key
        PRINT   msg_crlf
        rts

;=============================================================================
; Exceptions
;=============================================================================
exc_entry:
        tst.l   catch_pc
        beq.s   .report
        move.l  catch_sp,sp
        move.l  catch_pc,-(sp)
        clr.l   catch_pc
        rts                             ; "longjmp" to the catch point

.report:
        ; frame: 0(sp) SR.w, 2(sp) PC.l, 6(sp) format/vector.w
        move.w  (sp),d6                 ; SR
        move.l  2(sp),d5                ; PC
        move.w  6(sp),d4
        move.w  d4,d3
        lsr.w   #8,d3
        lsr.w   #4,d3                   ; d3 = frame format
        and.l   #$0FFF,d4
        lsr.l   #2,d4                   ; d4 = vector number
        move.l  #-1,a3
        cmp.w   #$A,d3
        beq.s   .fault
        cmp.w   #$B,d3
        bne.s   .nofault
.fault: move.l  $10(sp),a3              ; data cycle fault address
.nofault:
        move.l  #$81,SYS_LEDS
        PRINT   msg_exc
        ; name lookup
        lea     exc_names,a0
        move.l  d4,d0
        cmp.l   #12,d0
        blo.s   .named
        lea     name_trap,a0
        cmp.l   #32,d0
        blo.s   .generic
        cmp.l   #48,d0
        blo.s   .print
.generic:
        lea     name_exc,a0
        bra.s   .print
.named: lsl.l   #2,d0
        move.l  (a0,d0.l),a0
.print: bsr     puts
        PRINT   msg_vec
        move.l  d4,d0
        bsr     putdec
        PRINT   msg_atpc
        move.l  d5,d0
        bsr     puthex8
        PRINT   msg_sr
        move.l  d6,d0
        moveq   #4,d1
        bsr     puthex
        cmp.l   #-1,a3
        beq.s   .noaddr
        PRINT   msg_addr
        move.l  a3,d0
        bsr     puthex8
.noaddr:
        PRINT   msg_crlf
        bra     monitor

exc_names:
        dc.l    name_exc,name_exc,name_berr,name_aerr
        dc.l    name_ill,name_div0,name_chk,name_trapv
        dc.l    name_priv,name_trace,name_linea,name_linef

;=============================================================================
; TRAP #15 system calls. d0 = function; registers other than results kept.
;   0  exit to monitor
;   1  putc     d1.b = char
;   2  getc     -> d1.l = char (waits)
;   3  puts     a0 = NUL-terminated string
;   4  poll     -> d1.l = 1 if a char is waiting, else 0
;   5  disk read   d1 = SCSI ID, d2 = LBA, d3 = blocks, a0 = buffer -> d0 = status (0 = ok)
;   6  disk write  same as 5
;   7  info     -> d1 = RAM size, d2 = bit0 FPU present, bit1 SCSI present
; Unknown functions return d0 = -1.
;=============================================================================
trap15:
        cmp.l   #7,d0
        bhi     .bad
        move.w  .tab(pc,d0.w*2),d0
        jmp     .tab(pc,d0.w)
.tab:   dc.w    .exit-.tab,.putc-.tab,.getc-.tab,.puts-.tab
        dc.w    .poll-.tab,.dread-.tab,.dwrite-.tab,.info-.tab
.exit:  PRINT   msg_exit
        bra     monitor
.putc:  move.l  d1,d0
        bsr     putc
        moveq   #0,d0
        rte
.getc:  bsr     getc
        move.l  d0,d1
        moveq   #0,d0
        rte
.puts:  move.l  a0,-(sp)
        bsr     puts
        move.l  (sp)+,a0
        moveq   #0,d0
        rte
.poll:  moveq   #0,d1
        move.l  UART_STATUS,d0
        btst    #1,d0
        beq.s   .p0
        moveq   #1,d1
.p0:    moveq   #0,d0
        rte
.dread: moveq   #$28,d0
        bra.s   .disk
.dwrite:
        moveq   #$2A,d0
.disk:  movem.l d1-d4/a1,-(sp)
        move.b  d0,d4
        move.l  a0,a1
        move.l  d1,d0
        and.l   #7,d0
        move.l  d2,d1
        move.l  d3,d2
        bsr     scsi_rw
        and.l   #SCSI_ERRMASK,d0
        movem.l (sp)+,d1-d4/a1
        rte
.info:  move.l  ram_top,d1
        moveq   #0,d2
        tst.b   fpu_flag
        beq.s   .i1
        bset    #0,d2
.i1:    tst.b   scsi_ok
        beq.s   .i2
        bset    #1,d2
.i2:    moveq   #0,d0
        rte
.bad:   moveq   #-1,d0
        rte

;=============================================================================
; Console I/O
;=============================================================================
; putc: d0.b -> UART
putc:
        move.l  d1,-(sp)
.wait:  move.l  UART_STATUS,d1
        btst    #0,d1
        beq.s   .wait
        moveq   #0,d1
        move.b  d0,d1
        move.l  d1,UART_DATA
        move.l  (sp)+,d1
        rts

; getc: -> d0.l char (waits)
getc:
        move.l  UART_STATUS,d0
        btst    #1,d0
        beq.s   getc
        move.l  UART_DATA,d0
        and.l   #$FF,d0
        rts

; puts: a0 -> NUL-terminated string. a0 is advanced past it.
puts:
        move.l  d0,-(sp)
.l:     move.b  (a0)+,d0
        beq.s   .done
        bsr     putc
        bra.s   .l
.done:  move.l  (sp)+,d0
        rts

; puthex: print the low d1 nibbles of d0. Preserves d0-d2.
puthex8:
        move.l  d1,-(sp)
        moveq   #8,d1
        bsr.s   puthex
        move.l  (sp)+,d1
        rts
puthex2:
        move.l  d1,-(sp)
        moveq   #2,d1
        bsr.s   puthex
        move.l  (sp)+,d1
        rts
puthex:
        movem.l d0-d2/a0,-(sp)
        move.l  d0,d2
        moveq   #8,d0
        sub.l   d1,d0
        lsl.l   #2,d0
        rol.l   d0,d2                   ; first wanted nibble now at the top
        subq.l  #1,d1
        lea     hexdigits,a0
.l:     rol.l   #4,d2
        move.l  d2,d0
        and.l   #$F,d0
        move.b  (a0,d0.l),d0
        bsr     putc
        dbra    d1,.l
        movem.l (sp)+,d0-d2/a0
        rts

; putdec: print d0 as unsigned decimal. Preserves d0-d2.
putdec:
        movem.l d0-d2,-(sp)
        moveq   #0,d2
.div:   divul.l #10,d1:d0
        move.w  d1,-(sp)
        addq.l  #1,d2
        tst.l   d0
        bne.s   .div
.out:   move.w  (sp)+,d0
        add.b   #'0',d0
        bsr     putc
        subq.l  #1,d2
        bne.s   .out
        movem.l (sp)+,d0-d2
        rts

; getline: read a line with echo and backspace into linebuf.
; -> a0 = linebuf (NUL-terminated). Clobbers d0-d1.
getline:
        lea     linebuf,a0
        moveq   #0,d1
.loop:  bsr     getc
        cmp.b   #13,d0
        beq.s   .end
        cmp.b   #10,d0
        beq.s   .end
        cmp.b   #8,d0
        beq.s   .bs
        cmp.b   #127,d0
        beq.s   .bs
        cmp.b   #' ',d0
        blo.s   .loop
        cmp.w   #LINEMAX-1,d1
        bhs.s   .loop
        move.b  d0,(a0,d1.w)
        addq.w  #1,d1
        bsr     putc
        bra.s   .loop
.bs:    tst.w   d1
        beq.s   .loop
        subq.w  #1,d1
        move.l  a0,-(sp)
        PRINT   msg_bs
        move.l  (sp)+,a0
        bra.s   .loop
.end:   clr.b   (a0,d1.w)
        move.l  a0,-(sp)
        PRINT   msg_crlf
        move.l  (sp)+,a0
        rts

; skipsp: advance a0 past spaces
skipsp:
        cmp.b   #' ',(a0)
        bne.s   .done
        addq.l  #1,a0
        bra.s   skipsp
.done:  rts

; gethex: parse a hex number at a0 (optional '$'). -> d0 value, d1 digit
; count (0 = none). Clobbers d2.
gethex:
        bsr.s   skipsp
        moveq   #0,d0
        moveq   #0,d1
        cmp.b   #'$',(a0)
        bne.s   .l
        addq.l  #1,a0
.l:     move.b  (a0),d2
        bsr.s   hexval
        bmi.s   .done
        lsl.l   #4,d0
        or.b    d2,d0
        addq.l  #1,d1
        addq.l  #1,a0
        bra.s   .l
.done:  rts

; hexval: d2.b ASCII -> d2.l nibble value, N flag set if not a hex digit
hexval:
        cmp.b   #'a',d2
        blo.s   .up
        sub.b   #32,d2
.up:    cmp.b   #'0',d2
        blo.s   .bad
        cmp.b   #'9',d2
        bls.s   .dig
        cmp.b   #'A',d2
        blo.s   .bad
        cmp.b   #'F',d2
        bhi.s   .bad
        sub.b   #'A'-10,d2
        bra.s   .ok
.dig:   sub.b   #'0',d2
.ok:    and.l   #$F,d2
        rts
.bad:   moveq   #-1,d2
        rts

;=============================================================================
; SCSI
;=============================================================================
; scsi_exec: d0 = target ID, d1 = CDB length, a1 = DMA buffer, d2 = DMA length.
; CDB taken from `cdb`. -> d0 = SCSI STATUS register.
scsi_exec:
        move.l  d0,SCSI_TARGET
        move.l  a1,SCSI_DMA_ADDR
        move.l  d2,SCSI_DMA_LEN
        move.l  d1,SCSI_CDB_LEN
        move.l  cdb,SCSI_CDB
        move.l  cdb+4,SCSI_CDB+4
        move.l  cdb+8,SCSI_CDB+8
        move.l  cdb+12,SCSI_CDB+12
        move.l  #1,SCSI_CMD
.wait:  move.l  SCSI_STATUS,d0
        btst    #1,d0
        beq.s   .wait
        rts

clear_cdb:
        clr.l   cdb
        clr.l   cdb+4
        clr.l   cdb+8
        clr.l   cdb+12
        rts

; scsi_inquiry: d0 = ID -> scsibuf, d0 = status
scsi_inquiry:
        move.l  d0,-(sp)
        bsr     clear_cdb
        move.b  #$12,cdb
        move.b  #36,cdb+4
        move.l  (sp)+,d0
        moveq   #6,d1
        lea     scsibuf,a1
        moveq   #36,d2
        bra     scsi_exec

; scsi_capacity: d0 = ID -> d0 = status, d1 = block count
scsi_capacity:
        move.l  d0,-(sp)
        bsr     clear_cdb
        move.b  #$25,cdb
        move.l  (sp)+,d0
        moveq   #10,d1
        lea     scsibuf,a1
        moveq   #8,d2
        bsr     scsi_exec
        move.l  scsibuf,d1
        addq.l  #1,d1
        rts

; scsi_rw: d0 = ID, d1 = LBA, d2 = blocks, a1 = buffer, d4.b = $28 read / $2A write
; -> d0 = status
scsi_rw:
        move.l  d0,-(sp)
        bsr     clear_cdb
        move.b  d4,cdb
        move.l  d1,cdb+2
        move.w  d2,cdb+7
        move.l  d2,d0
        moveq   #9,d1
        lsl.l   d1,d0
        move.l  d0,d2                   ; bytes
        move.l  (sp)+,d0
        moveq   #10,d1
        bra     scsi_exec

; scsi_scan: list every responding target
scsi_scan:
        moveq   #0,d7
.next:  cmp.l   #HOST_ID,d7
        beq     .skip
        move.l  d7,d0
        bsr     scsi_inquiry
        and.l   #SCSI_ERRMASK,d0
        bne     .skip
        PRINT   msg_id
        move.l  d7,d0
        bsr     putdec
        PRINT   msg_colon
        lea     scsibuf+8,a2
        moveq   #28-1,d3
.name:  move.b  (a2)+,d0
        bsr     putc
        dbra    d3,.name
        move.l  d7,d0
        bsr     scsi_capacity
        and.l   #SCSI_ERRMASK,d0
        bne.s   .nocap
        move.b  #' ',d0
        bsr     putc
        move.b  #' ',d0
        bsr     putc
        move.l  d1,d0
        moveq   #9,d1
        lsl.l   d1,d0                   ; bytes (fine below 4 GB)
        bsr     putsize
.nocap: move.l  d7,d0
        bsr     is_bootable
        tst.l   d0
        beq.s   .eol
        PRINT   msg_bootable
.eol:   PRINT   msg_crlf
.skip:  addq.l  #1,d7
        cmp.l   #8,d7
        blo     .next
        rts

; is_bootable: d0 = ID -> d0 = 1 if block 0 carries the boot magic. Leaves
; the boot block in scsibuf.
is_bootable:
        moveq   #0,d1
        moveq   #1,d2
        lea     scsibuf,a1
        moveq   #$28,d4
        bsr     scsi_rw
        and.l   #SCSI_ERRMASK,d0
        bne.s   .no
        cmp.l   #$415A3330,scsibuf      ; "AZ30"
        bne.s   .no
        cmp.l   #$424F4F54,scsibuf+4    ; "BOOT"
        bne.s   .no
        moveq   #1,d0
        rts
.no:    moveq   #0,d0
        rts

; find_boot: -> d0 = lowest bootable ID, or -1
find_boot:
        moveq   #0,d7
.l:     cmp.l   #HOST_ID,d7
        beq.s   .n
        move.l  d7,d0
        bsr.s   is_bootable
        tst.l   d0
        bne.s   .found
.n:     addq.l  #1,d7
        cmp.l   #8,d7
        blo.s   .l
        moveq   #-1,d0
        rts
.found: move.l  d7,d0
        rts

; boot_disk: d0 = ID. Loads and runs the boot program; returns (to the
; caller) only on failure.
boot_disk:
        move.l  d0,d7
        bsr.s   is_bootable
        tst.l   d0
        bne.s   .ok
        PRINT   msg_notboot
        rts
.ok:    move.l  scsibuf+8,a4            ; load address
        move.l  scsibuf+12,a5           ; entry
        move.l  scsibuf+16,d6           ; blocks
        PRINT   msg_booting
        move.l  d7,d0
        bsr     putdec
        PRINT   msg_load
        move.l  a4,d0
        bsr     puthex8
        PRINT   msg_entry
        move.l  a5,d0
        bsr     puthex8
        PRINT   msg_crlf
        cmp.l   #$FFFF,d6
        bhi.s   .fail
        move.l  d7,d0
        moveq   #1,d1
        move.l  d6,d2
        move.l  a4,a1
        moveq   #$28,d4
        bsr     scsi_rw
        and.l   #SCSI_ERRMASK,d0
        bne.s   .fail
        move.l  #$0F,SYS_LEDS
        move.l  d7,d0                   ; d0 = boot ID
        move.l  ram_top,d1              ; d1 = RAM size
        jsr     (a5)
        PRINT   msg_returned
        bsr     puthex8
        PRINT   msg_crlf
        bra     monitor
.fail:  PRINT   msg_ioerr
        rts

;=============================================================================
; Monitor
;=============================================================================
monitor:
        move.w  #$2700,sr
        move.l  ram_top,sp
        UNCATCH
        PRINT   msg_help_hint
mon_loop:
        move.l  ram_top,sp
        PRINT   msg_prompt
        bsr     getline
        bsr     skipsp
        move.b  (a0)+,d0
        beq.s   mon_loop
        cmp.b   #'a',d0
        blo.s   .up
        sub.b   #32,d0
.up:    move.b  (a0),d1                 ; second letter (for wl/rl/rd/wd)
        cmp.b   #'a',d1
        blo.s   .up2
        sub.b   #32,d1
.up2:
        cmp.b   #'?',d0
        beq     cmd_help
        cmp.b   #'H',d0
        beq     cmd_help
        cmp.b   #'D',d0
        beq     cmd_dump
        cmp.b   #'W',d0
        bne.s   .notw
        cmp.b   #'L',d1
        beq     cmd_wl
        cmp.b   #'D',d1
        beq     cmd_wd
        bra     cmd_write
.notw:  cmp.b   #'R',d0
        bne.s   .notr
        cmp.b   #'L',d1
        beq     cmd_rl
        cmp.b   #'D',d1
        beq     cmd_rd
        bra.s   .bad
.notr:  cmp.b   #'G',d0
        beq     cmd_go
        cmp.b   #'S',d0
        beq     cmd_srec
        cmp.b   #'B',d0
        beq     cmd_boot
        cmp.b   #'I',d0
        beq     cmd_info
        cmp.b   #'X',d0
        beq     cold_start
.bad:   PRINT   msg_what
        bra     mon_loop

cmd_help:
        PRINT   msg_help
        bra     mon_loop

need_arg:
        PRINT   msg_args
        bra     mon_loop

; d [addr] [len]
cmd_dump:
        bsr     gethex
        tst.l   d1
        beq.s   .noaddr
        move.l  d0,last_addr
.noaddr:
        bsr     gethex
        tst.l   d1
        bne.s   .len
        move.l  #$100,d0
.len:   move.l  d0,d6                   ; bytes remaining
        move.l  last_addr,a2
.row:   tst.l   d6
        beq     .done
        move.l  a2,d0
        bsr     puthex8
        PRINT   msg_colon
        moveq   #16-1,d3
        move.l  a2,a3
.hex:   move.b  (a3)+,d0
        bsr     puthex2
        move.b  #' ',d0
        bsr     putc
        dbra    d3,.hex
        move.b  #'|',d0
        bsr     putc
        moveq   #16-1,d3
.asc:   move.b  (a2)+,d0
        cmp.b   #' ',d0
        blo.s   .dot
        cmp.b   #'~',d0
        bls.s   .ch
.dot:   move.b  #'.',d0
.ch:    bsr     putc
        dbra    d3,.asc
        move.b  #'|',d0
        bsr     putc
        PRINT   msg_crlf
        sub.l   #16,d6
        bhi     .row
.done:  move.l  a2,last_addr
        bra     mon_loop

; w addr byte...
cmd_write:
        bsr     gethex
        tst.l   d1
        beq     need_arg
        move.l  d0,a2
.l:     bsr     gethex
        tst.l   d1
        beq     mon_loop
        move.b  d0,(a2)+
        bra.s   .l

; wl addr long...   (32-bit writes, needed for I/O registers)
cmd_wl:
        addq.l  #1,a0
        bsr     gethex
        tst.l   d1
        beq     need_arg
        move.l  d0,a2
.l:     bsr     gethex
        tst.l   d1
        beq     mon_loop
        move.l  d0,(a2)+
        bra.s   .l

; rl addr   (32-bit read)
cmd_rl:
        addq.l  #1,a0
        bsr     gethex
        tst.l   d1
        beq     need_arg
        move.l  d0,a2
        bsr     puthex8
        PRINT   msg_colon
        move.l  (a2),d0
        bsr     puthex8
        PRINT   msg_crlf
        bra     mon_loop

; g [addr]
cmd_go:
        bsr     gethex
        tst.l   d1
        bne.s   .have
        move.l  last_entry,d0
        bne.s   .have
        bra     need_arg
.have:  move.l  d0,a2
        move.l  ram_top,d1
        jsr     (a2)
        PRINT   msg_returned
        bsr     puthex8
        PRINT   msg_crlf
        bra     mon_loop

; i  - system info + SCSI scan
cmd_info:
        bsr     print_sysinfo
        tst.b   scsi_ok
        beq     mon_loop
        bsr     scsi_scan
        bra     mon_loop

; b [id]
cmd_boot:
        tst.b   scsi_ok
        beq.s   .noscsi
        bsr     gethex
        tst.l   d1
        bne.s   .id
        bsr     find_boot
        tst.l   d0
        bpl.s   .id
        PRINT   msg_nobootdisk
        bra     mon_loop
.id:    and.l   #7,d0
        bsr     boot_disk
        bra     mon_loop
.noscsi:
no_scsi:
        PRINT   msg_noscsi
        bra     mon_loop

; rd id lba addr [count] / wd id lba addr [count]
cmd_rd:
        moveq   #$28,d4
        bra.s   disk_cmd
cmd_wd:
        moveq   #$2A,d4
disk_cmd:
        tst.b   scsi_ok
        beq     no_scsi
        addq.l  #1,a0
        bsr     gethex
        tst.l   d1
        beq     need_arg
        move.l  d0,d5                   ; id
        bsr     gethex
        tst.l   d1
        beq     need_arg
        move.l  d0,d6                   ; lba
        bsr     gethex
        tst.l   d1
        beq     need_arg
        move.l  d0,a1                   ; buffer
        bsr     gethex
        tst.l   d1
        bne.s   .cnt
        moveq   #1,d0
.cnt:   move.l  d0,d2
        move.l  d5,d0
        and.l   #7,d0
        move.l  d6,d1
        bsr     scsi_rw
        bsr     report_status
        bra     mon_loop

; report_status: d0 = SCSI status register; prints OK or the error
report_status:
        move.l  d0,d1
        and.l   #SCSI_ERRMASK,d1
        bne.s   .err
        PRINT   msg_ok
        move.l  SCSI_XFER,d0
        bsr     putdec
        PRINT   msg_bytes
        rts
.err:   PRINT   msg_scsierr
        bsr     puthex8
        PRINT   msg_crlf
        rts

;-----------------------------------------------------------------------------
; s - load Motorola S-records from the console (S1/S2/S3 data, S7/S8/S9 end)
;-----------------------------------------------------------------------------
cmd_srec:
        PRINT   msg_srec
        moveq   #0,d7                   ; record count
.line:  bsr     getc
        cmp.b   #3,d0                   ; Ctrl-C aborts
        beq     .abort
        cmp.b   #'S',d0
        bne.s   .line
        bsr     getc
        move.b  d0,d6                   ; record type
        moveq   #0,d5                   ; checksum
        bsr     srec_byte
        move.l  d0,d4                   ; count (addr + data + checksum)
        moveq   #2,d3
        cmp.b   #'1',d6
        beq.s   .addr
        cmp.b   #'9',d6
        beq.s   .addr
        moveq   #3,d3
        cmp.b   #'2',d6
        beq.s   .addr
        cmp.b   #'8',d6
        beq.s   .addr
        moveq   #4,d3
        cmp.b   #'3',d6
        beq.s   .addr
        cmp.b   #'7',d6
        beq.s   .addr
        bra.s   .line                   ; S0/S5/S6: ignore rest of the line
.addr:  sub.l   d3,d4
        subq.l  #1,d4                   ; d4 = data bytes
        bmi.s   .bad
        move.l  #0,a2
        subq.l  #1,d3
.a:     bsr     srec_byte
        move.l  a2,d1
        lsl.l   #8,d1
        or.b    d0,d1
        move.l  d1,a2
        dbra    d3,.a
        cmp.b   #'4',d6
        bhs.s   .term
        tst.l   d4
        beq.s   .sum
        subq.l  #1,d4
.d:     bsr     srec_byte
        move.b  d0,(a2)+
        dbra    d4,.d
.sum:   bsr     srec_byte
        cmp.b   #$FF,d5
        bne.s   .bad
        addq.l  #1,d7
        bra     .line
.term:  bsr     srec_byte
        cmp.b   #$FF,d5
        bne.s   .bad
        move.l  a2,last_entry
        PRINT   msg_srec_done
        move.l  d7,d0
        bsr     putdec
        PRINT   msg_srec_entry
        move.l  a2,d0
        bsr     puthex8
        PRINT   msg_crlf
        bra     mon_loop
.bad:   PRINT   msg_srec_bad
        bra     mon_loop
.abort: PRINT   msg_crlf
        bra     mon_loop

; srec_byte: two hex chars from the console -> d0.l, added into d5
srec_byte:
        move.l  d2,-(sp)
        bsr     getc
        move.b  d0,d2
        bsr     hexval
        move.l  d2,d1
        lsl.l   #4,d1
        bsr     getc
        move.b  d0,d2
        bsr     hexval
        and.l   #$F,d2
        or.l    d2,d1
        and.l   #$FF,d1
        add.b   d1,d5
        move.l  d1,d0
        move.l  (sp)+,d2
        rts

;=============================================================================
; Strings
;=============================================================================
hexdigits:      dc.b    "0123456789ABCDEF"
msg_banner:     dc.b    13,10,"az030 boot ROM v0.1",13,10,0
msg_cpu:        dc.b    "CPU:  MC68030",13,10,"FPU:  ",0
msg_fpu_yes:    dc.b    "MC6888x",13,10,0
msg_fpu_no:     dc.b    "none",13,10,0
msg_ram:        dc.b    "RAM:  ",0
msg_scsi:       dc.b    "SCSI: ",0
msg_present:    dc.b    "present",13,10,0
msg_absent:     dc.b    "not present",13,10,0
msg_mb:         dc.b    " MB",0
msg_kb:         dc.b    " KB",0
msg_crlf:       dc.b    13,10,0
msg_bs:         dc.b    8," ",8,0
msg_colon:      dc.b    ": ",0
msg_scsi_scan:  dc.b    13,10,"Scanning SCSI bus...",13,10,0
msg_id:         dc.b    "  ID ",0
msg_bootable:   dc.b    "  [bootable]",0
msg_autoboot1:  dc.b    13,10,"Booting SCSI ID ",0
msg_autoboot2:  dc.b    " - press any key for the monitor... ",0
msg_booting:    dc.b    "Loading from SCSI ID ",0
msg_load:       dc.b    ": load $",0
msg_entry:      dc.b    ", entry $",0
msg_returned:   dc.b    13,10,"Program returned, d0 = $",0
msg_exit:       dc.b    13,10,"Program exited.",13,10,0
msg_notboot:    dc.b    "Disk is not bootable (no AZ30BOOT block).",13,10,0
msg_nobootdisk: dc.b    "No bootable disk found.",13,10,0
msg_ioerr:      dc.b    "Disk read error.",13,10,0
msg_noscsi:     dc.b    "No SCSI controller.",13,10,0
msg_ok:         dc.b    "OK, ",0
msg_bytes:      dc.b    " bytes",13,10,0
msg_scsierr:    dc.b    "SCSI error, status $",0
msg_exc:        dc.b    13,10,"*** ",0
msg_vec:        dc.b    " (vector ",0
msg_atpc:       dc.b    ") at PC=$",0
msg_sr:         dc.b    " SR=$",0
msg_addr:       dc.b    " address=$",0
msg_help_hint:  dc.b    13,10,"Monitor ready. Type ? for help.",13,10,0
msg_prompt:     dc.b    "az030> ",0
msg_what:       dc.b    "Unknown command. Type ? for help.",13,10,0
msg_args:       dc.b    "Missing argument. Type ? for help.",13,10,0
msg_srec:       dc.b    "Send S-records (Ctrl-C to abort)...",13,10,0
msg_srec_done:  dc.b    "Loaded ",0
msg_srec_entry: dc.b    " records, entry $",0
msg_srec_bad:   dc.b    13,10,"S-record checksum/format error.",13,10,0
msg_help:
        dc.b    "Numbers are hex.",13,10
        dc.b    "  d [addr] [len]          dump memory",13,10
        dc.b    "  w addr byte...          write bytes",13,10
        dc.b    "  wl addr long...         write longs (use for I/O registers)",13,10
        dc.b    "  rl addr                 read a long",13,10
        dc.b    "  g [addr]                call addr (default: last S-record entry)",13,10
        dc.b    "  s                       load S-records from the console",13,10
        dc.b    "  b [id]                  boot from SCSI disk",13,10
        dc.b    "  rd id lba addr [count]  read disk blocks into memory",13,10
        dc.b    "  wd id lba addr [count]  write memory to disk blocks",13,10
        dc.b    "  i                       system info / SCSI scan",13,10
        dc.b    "  x                       restart the ROM",13,10,0

name_exc:       dc.b    "Exception",0
name_berr:      dc.b    "Bus error",0
name_aerr:      dc.b    "Address error",0
name_ill:       dc.b    "Illegal instruction",0
name_div0:      dc.b    "Divide by zero",0
name_chk:       dc.b    "CHK",0
name_trapv:     dc.b    "TRAPV",0
name_priv:      dc.b    "Privilege violation",0
name_trace:     dc.b    "Trace",0
name_linea:     dc.b    "Line A",0
name_linef:     dc.b    "Line F",0
name_trap:      dc.b    "TRAP",0
        even

        ifgt    *-rom_start-ROM_SIZE
        fail    "boot ROM exceeds 16 KB"
        endif
        dcb.b   ROM_SIZE-(*-rom_start),$FF
