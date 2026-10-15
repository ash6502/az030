; VERA demo for the az030 emulator. Run the emulator with --vera, then boot this from
; a SCSI disk (see the Makefile/README) or load it at $10000 and "g 10000".
;
; Shows: an 80x60 text layer with an uploaded font, a 4bpp tile layer scrolled from
; the VSYNC interrupt, eight sprites at two Z-depths with collision masks, a palette
; write, and a border that changes colour at line 240 through the LINE interrupt.
; Keys: z = toggle 2x zoom, c = toggle composite (chroma off), q = quit to the monitor.
;
; VERA registers are at $FE003000 + 4*n and are accessed as longs (data in the low
; byte). The VERA interrupt is autovectored at level 3 (vector 27).

        org     $10000

VERA            equ     $FE003000
V_ADDR_L        equ     VERA+$00
V_ADDR_M        equ     VERA+$04
V_ADDR_H        equ     VERA+$08
V_DATA0         equ     VERA+$0C
V_DATA1         equ     VERA+$10
V_CTRL          equ     VERA+$14
V_IEN           equ     VERA+$18
V_ISR           equ     VERA+$1C
V_IRQLINE       equ     VERA+$20
V_DC_VIDEO      equ     VERA+$24        ; DCSEL=1: DC_HSTART
V_DC_HSCALE     equ     VERA+$28        ; DCSEL=1: DC_HSTOP
V_DC_VSCALE     equ     VERA+$2C        ; DCSEL=1: DC_VSTART
V_DC_BORDER     equ     VERA+$30        ; DCSEL=1: DC_VSTOP
V_L0_CONFIG     equ     VERA+$34
V_L0_MAPBASE    equ     VERA+$38
V_L0_TILEBASE   equ     VERA+$3C
V_L0_HSCROLL_L  equ     VERA+$40
V_L0_HSCROLL_H  equ     VERA+$44
V_L0_VSCROLL_L  equ     VERA+$48
V_L0_VSCROLL_H  equ     VERA+$4C
V_L1_CONFIG     equ     VERA+$50
V_L1_MAPBASE    equ     VERA+$54
V_L1_TILEBASE   equ     VERA+$58

; VRAM layout
TEXT_MAP        equ     $00000          ; 128x64 entries
L0_MAP          equ     $04000          ; 32x32 entries
FONT            equ     $0F800          ; 256 chars x 8 bytes
L0_TILES        equ     $10000          ; 4bpp 8x8 tiles, 32 bytes each
BALL            equ     $12000          ; 16x16 4bpp sprite image
PALETTE         equ     $1FA00
SPRITES         equ     $1FC00

NSPR            equ     8
VEC_VERA        equ     27              ; level 3 autovector

start:
        move.w  #$2700,sr
        ; private vector table: a copy of the ROM's, plus the VERA handler
        movec   vbr,a0
        move.l  a0,old_vbr
        lea     vectors,a1
        move.w  #256-1,d0
.cpv:   move.l  (a0)+,(a1)+
        dbra    d0,.cpv
        move.l  #vera_irq,vectors+VEC_VERA*4
        lea     vectors,a0
        movec   a0,vbr

        move.l  #$80,V_CTRL             ; reset the VERA
        move.l  #0,V_CTRL

        ; text map: spaces, white on transparent
        moveq   #TEXT_MAP,d0
        bsr     vset
        move.w  #128*64-1,d1
.clr:   move.l  #' ',V_DATA0
        move.l  #$01,V_DATA0
        dbra    d1,.clr

        ; font for ASCII 32-127
        move.l  #FONT+32*8,d0
        bsr     vset
        lea     font,a0
        move.w  #96*8-1,d1
.fnt:   moveq   #0,d0
        move.b  (a0)+,d0
        move.l  d0,V_DATA0
        dbra    d1,.fnt

        ; layer 0 tiles: tile 0 empty, tile 1 diagonal stripes in colours 1-8
        move.l  #L0_TILES,d0
        bsr     vset
        moveq   #32-1,d1
.t0:    move.l  #0,V_DATA0
        dbra    d1,.t0
        moveq   #0,d2                   ; y
.ty:    moveq   #0,d3                   ; x, two pixels per byte
.tx:    move.l  d2,d0
        add.l   d3,d0
        and.l   #7,d0
        addq.l  #1,d0
        lsl.l   #4,d0
        move.l  d2,d4
        add.l   d3,d4
        addq.l  #1,d4
        and.l   #7,d4
        addq.l  #1,d4
        or.l    d4,d0
        move.l  d0,V_DATA0
        addq.l  #2,d3
        cmp.l   #8,d3
        blo.s   .tx
        addq.l  #1,d2
        cmp.l   #8,d2
        blo.s   .ty

        ; layer 0 map: checkerboard of tile 1, palette offset by row
        move.l  #L0_MAP,d0
        bsr     vset
        moveq   #0,d2                   ; tile row
.my:    moveq   #0,d3                   ; tile column
.mx:    move.l  d2,d0
        add.l   d3,d0
        and.l   #1,d0
        move.l  d0,V_DATA0              ; tile index
        move.l  d2,d0
        and.l   #7,d0
        addq.l  #2,d0
        lsl.l   #4,d0
        move.l  d0,V_DATA0              ; palette offset
        addq.l  #1,d3
        cmp.l   #32,d3
        blo.s   .mx
        addq.l  #1,d2
        cmp.l   #32,d2
        blo.s   .my

        ; sprite image: a shaded ball (colours 3 centre, 2, 1 rim)
        move.l  #BALL,d0
        bsr     vset
        moveq   #0,d2
.by:    moveq   #0,d3
.bx:    move.l  d3,d0
        bsr     ballpx
        lsl.l   #4,d0
        move.l  d0,d5
        move.l  d3,d0
        addq.l  #1,d0
        bsr     ballpx
        or.l    d5,d0
        move.l  d0,V_DATA0
        addq.l  #2,d3
        cmp.l   #16,d3
        blo.s   .bx
        addq.l  #1,d2
        cmp.l   #16,d2
        blo.s   .by

        ; palette entry 11: dark blue (used as the text background)
        move.l  #PALETTE+11*2,d0
        bsr     vset
        move.l  #$24,V_DATA0            ; green 2, blue 4
        move.l  #$00,V_DATA0            ; red 0

        bsr     update_sprites

        ; layer 1: 1bpp text, 16 colours, map 128x64, 8x8 font
        move.l  #$60,V_L1_CONFIG
        move.l  #TEXT_MAP>>9,V_L1_MAPBASE
        move.l  #(FONT>>11)<<2,V_L1_TILEBASE
        ; layer 0: 4bpp tiles, map 32x32, 8x8 tiles
        move.l  #$02,V_L0_CONFIG
        move.l  #L0_MAP>>9,V_L0_MAPBASE
        move.l  #(L0_TILES>>11)<<2,V_L0_TILEBASE
        ; active area 16..624 x 16..464, so the border shows
        move.l  #2,V_CTRL               ; DCSEL=1
        move.l  #16>>2,V_DC_VIDEO
        move.l  #624>>2,V_DC_HSCALE
        move.l  #16>>1,V_DC_VSCALE
        move.l  #464>>1,V_DC_BORDER
        move.l  #0,V_CTRL
        move.l  #128,V_DC_HSCALE
        move.l  #128,V_DC_VSCALE
        move.l  #6,V_DC_BORDER
        move.l  video_mode,V_DC_VIDEO   ; sprites + both layers, VGA

        ; text
        lea     texts,a2
.txt:   move.w  (a2)+,d0                ; column
        bmi.s   .txtend
        move.w  (a2)+,d1                ; row
        move.w  (a2)+,d2                ; colour
        ext.l   d0
        ext.l   d1
        ext.l   d2
        move.l  (a2)+,a0
        bsr     vtext
        bra.s   .txt
.txtend:
        ; colour bar: the 16 text colours as backgrounds
        move.l  #11*256+2*2,d0          ; row 11, column 2
        bsr     vset
        moveq   #0,d2
.bar:   move.l  d2,d1
        lsl.l   #4,d1
        moveq   #4-1,d3
.bar1:  move.l  #' ',V_DATA0
        move.l  d1,V_DATA0
        dbra    d3,.bar1
        addq.l  #1,d2
        cmp.l   #16,d2
        blo.s   .bar

        ; interrupts: LINE at 240 and VSYNC
        move.l  #240,V_IRQLINE
        move.l  #3,V_IEN
        move.w  #$2200,sr               ; let level 3 in

main:   move.l  frames,d0
        bsr     hex8
        lea     hexbuf,a0
        moveq   #16,d0
        moveq   #56,d1
        move.l  #$B7,d2
        bsr     vtext
        move.l  collisions,d0
        bsr     hex8
        lea     hexbuf+7,a0             ; one digit
        moveq   #37,d0
        moveq   #56,d1
        move.l  #$B7,d2
        bsr     vtext

        moveq   #4,d0                   ; key waiting?
        trap    #15
        tst.l   d1
        beq     main
        moveq   #2,d0
        trap    #15
        cmp.b   #'q',d1
        beq.s   quit
        cmp.b   #'z',d1
        bne.s   .notz
        eor.l   #$C0,zoom               ; 128 <-> 64
        move.l  zoom,V_DC_HSCALE
        move.l  zoom,V_DC_VSCALE
        bra     main
.notz:  cmp.b   #'c',d1
        bne     main
        eor.l   #$07,video_mode         ; VGA <-> NTSC with chroma disabled
        move.l  video_mode,V_DC_VIDEO
        bra     main

quit:   move.w  #$2700,sr
        move.l  #0,V_IEN
        move.l  #7,V_ISR
        move.l  old_vbr,a0
        movec   a0,vbr
        moveq   #0,d0                   ; exit to the monitor
        trap    #15

;-----------------------------------------------------------------------------
; VERA interrupt: LINE recolours the border; VSYNC scrolls layer 0, moves the
; sprites and records the sprite collisions.
;-----------------------------------------------------------------------------
vera_irq:
        movem.l d0-d4/a0-a1,-(sp)
        move.l  V_CTRL,d4               ; the main program's ADDRSEL
        move.l  V_ISR,d0
        btst    #1,d0
        beq.s   .noline
        move.l  #2,V_ISR
        move.l  #2,V_DC_BORDER          ; red below line 240
.noline:
        btst    #0,d0
        beq.s   .done
        move.l  #5,V_ISR                ; ack VSYNC and SPRCOL
        move.l  #6,V_DC_BORDER          ; blue from the top
        lsr.l   #4,d0
        and.l   #$F,d0
        move.l  d0,collisions
        addq.l  #1,frames
        move.l  frames,d1
        move.l  d1,V_L0_HSCROLL_L
        lsr.l   #8,d1
        move.l  d1,V_L0_HSCROLL_H
        move.l  frames,d1
        lsr.l   #1,d1
        move.l  d1,V_L0_VSCROLL_L
        lsr.l   #8,d1
        move.l  d1,V_L0_VSCROLL_H
        bsr.s   update_sprites
.done:  move.l  d4,V_CTRL
        movem.l (sp)+,d0-d4/a0-a1
        rte

; Move the sprites and rewrite their attributes through data port 1.
update_sprites:
        move.l  #1,V_CTRL               ; ADDRSEL=1
        move.l  #SPRITES&$FF,V_ADDR_L
        move.l  #(SPRITES>>8)&$FF,V_ADDR_M
        move.l  #$11,V_ADDR_H           ; increment 1, bit 16
        lea     sprites,a0
        lea     sprattr,a1
        moveq   #NSPR-1,d3
.spr:   move.l  (a0),d1                 ; x += dx, bounce at the edges
        add.l   8(a0),d1
        cmp.l   #624,d1
        bls.s   .xok
        neg.l   8(a0)
        move.l  (a0),d1
        add.l   8(a0),d1
.xok:   move.l  d1,(a0)
        move.l  4(a0),d1                ; y += dy
        add.l   12(a0),d1
        cmp.l   #464,d1
        bls.s   .yok
        neg.l   12(a0)
        move.l  4(a0),d1
        add.l   12(a0),d1
.yok:   move.l  d1,4(a0)
        move.l  #(BALL>>5)&$FF,V_DATA1
        move.l  #(BALL>>13)&$F,V_DATA1  ; 4bpp
        move.l  (a0),d1
        move.l  d1,V_DATA1
        lsr.l   #8,d1
        move.l  d1,V_DATA1
        move.l  4(a0),d1
        move.l  d1,V_DATA1
        lsr.l   #8,d1
        move.l  d1,V_DATA1
        moveq   #0,d1
        move.b  (a1)+,d1
        move.l  d1,V_DATA1              ; collision mask, Z, flips
        move.b  (a1)+,d1
        move.l  d1,V_DATA1              ; size, palette offset
        lea     16(a0),a0
        dbra    d3,.spr
        move.l  #0,V_CTRL
        rts

;-----------------------------------------------------------------------------
; Helpers
;-----------------------------------------------------------------------------
; vset: point data port 0 at VRAM address d0, increment 1
vset:   move.l  d0,-(sp)
        move.l  d0,V_ADDR_L
        lsr.l   #8,d0
        move.l  d0,V_ADDR_M
        lsr.l   #8,d0
        and.l   #1,d0
        or.l    #$10,d0
        move.l  d0,V_ADDR_H
        move.l  (sp)+,d0
        rts

; vtext: string a0 at column d0, row d1, colour d2
vtext:  movem.l d0-d1/a0,-(sp)
        lsl.l   #8,d1
        add.l   d0,d0
        add.l   d1,d0
        bsr.s   vset
.l:     moveq   #0,d1
        move.b  (a0)+,d1
        beq.s   .done
        move.l  d1,V_DATA0
        move.l  d2,V_DATA0
        bra.s   .l
.done:  movem.l (sp)+,d0-d1/a0
        rts

; hex8: d0 -> hexbuf as 8 hex digits
hex8:   movem.l d0-d2/a0-a1,-(sp)
        lea     hexbuf+8,a0
        lea     hexdigits,a1
        moveq   #8-1,d2
.h:     move.l  d0,d1
        and.l   #$F,d1
        move.b  (a1,d1.l),-(a0)
        lsr.l   #4,d0
        dbra    d2,.h
        movem.l (sp)+,d0-d2/a0-a1
        rts

; ballpx: colour of ball pixel x = d0, y = d2
ballpx: movem.l d1/d4,-(sp)
        add.l   d0,d0
        sub.l   #15,d0
        muls.w  d0,d0
        move.l  d2,d1
        add.l   d1,d1
        sub.l   #15,d1
        muls.w  d1,d1
        add.l   d1,d0                   ; (2x-15)^2 + (2y-15)^2
        moveq   #3,d4
        cmp.l   #64,d0
        blo.s   .c
        moveq   #2,d4
        cmp.l   #144,d0
        blo.s   .c
        moveq   #1,d4
        cmp.l   #225,d0
        blo.s   .c
        moveq   #0,d4
.c:     move.l  d4,d0
        movem.l (sp)+,d1/d4
        rts

;-----------------------------------------------------------------------------
; Data
;-----------------------------------------------------------------------------
        cnop    0,4
frames:         dc.l    0
collisions:     dc.l    0
old_vbr:        dc.l    0
zoom:           dc.l    128
video_mode:     dc.l    $71             ; sprites, layer 1, layer 0, VGA

; x, y, dx, dy per sprite
sprites:
        dc.l    40,40,3,2
        dc.l    100,300,-2,3
        dc.l    300,100,2,-2
        dc.l    500,350,-3,-1
        dc.l    200,200,1,3
        dc.l    400,60,-1,2
        dc.l    560,420,2,-3
        dc.l    80,400,3,-2

; attribute bytes 6 and 7: sprites 0-3 in front (Z 3), 4-7 behind layer 0 (Z 1).
; Collision masks: front group %0001, back group %0010, sprite 0 in both.
sprattr:
        dc.b    $3C,$52, $1C,$53, $1C,$54, $1C,$55
        dc.b    $24,$56, $24,$57, $24,$58, $24,$59

texts:  dc.w    2,2,$61
        dc.l    t_title
        dc.w    2,4,$B1
        dc.l    t_l1
        dc.w    2,5,$B7
        dc.l    t_l0
        dc.w    2,6,$B3
        dc.l    t_spr
        dc.w    2,7,$BD
        dc.l    t_border
        dc.w    2,9,$B1
        dc.l    t_keys
        dc.w    2,56,$B1
        dc.l    t_status
        dc.w    -1

t_title:        dc.b    " AZ030 VERA DEMO ",0
t_l1:           dc.b    "LAYER 1: 80X60 TEXT, 16 COLOURS, 1BPP 8X8 FONT",0
t_l0:           dc.b    "LAYER 0: 4BPP TILES, SCROLLED FROM THE VSYNC IRQ",0
t_spr:          dc.b    "SPRITES: 4 IN FRONT OF THE LAYERS, 4 BEHIND LAYER 0",0
t_border:       dc.b    "BORDER: BLUE ABOVE LINE 240, RED BELOW (LINE IRQ)",0
t_keys:         dc.b    "KEYS: Z = 2X ZOOM   C = COMPOSITE MONO   Q = QUIT",0
t_status:       dc.b    "FRAME        $          COLLISIONS",0
hexdigits:      dc.b    "0123456789ABCDEF"
hexbuf:         dc.b    "00000000",0

        include "font.inc"

        cnop    0,4
vectors:        ds.l    256
