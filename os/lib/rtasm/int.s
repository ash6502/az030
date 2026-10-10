;=============================================================================
; Integer arithmetic helpers that LLVM calls on the 68030 (libgcc/compiler-rt
; names). 64-bit values are passed on the stack high word first and returned
; in d0 (high) : d1 (low).
;=============================================================================

        xdef    __mulsi3
        xdef    __divsi3
        xdef    __udivsi3
        xdef    __modsi3
        xdef    __umodsi3
        xdef    __muldi3
        xdef    __udivdi3
        xdef    __umoddi3
        xdef    __divdi3
        xdef    __moddi3
        xdef    __ashldi3
        xdef    __lshrdi3
        xdef    __ashrdi3
        xdef    __cmpdi2
        xdef    __ucmpdi2

        section .text

; ---- 32-bit ---------------------------------------------------------------
__mulsi3:
        move.l  4(sp),d0
        muls.l  8(sp),d0
        rts

__divsi3:
        move.l  4(sp),d0
        divs.l  8(sp),d0
        rts

__udivsi3:
        move.l  4(sp),d0
        divu.l  8(sp),d0
        rts

__modsi3:
        move.l  4(sp),d1
        divsl.l 8(sp),d0:d1
        rts

__umodsi3:
        move.l  4(sp),d1
        divul.l 8(sp),d0:d1
        rts

; ---- 64-bit multiply --------------------------------------------------------
; (ah:al) * (bh:bl) mod 2^64 = al*bl + ((ah*bl + al*bh) << 32)
__muldi3:
        movem.l d2-d3,-(sp)
        move.l  16(sp),d1               ; al
        mulu.l  24(sp),d0:d1            ; d0:d1 = al * bl
        move.l  12(sp),d2               ; ah
        mulu.l  24(sp),d2               ; ah * bl
        move.l  16(sp),d3               ; al
        mulu.l  20(sp),d3               ; al * bh
        add.l   d2,d0
        add.l   d3,d0
        movem.l (sp)+,d2-d3
        rts

; ---- 64-bit unsigned division core ------------------------------------------
; In:  d2:d3 dividend, d4:d5 divisor (non-zero)
; Out: d2:d3 quotient, d6:d7 remainder. Uses d0/d1.
udivmod64:
        tst.l   d4
        bne.s   .long
        ; divisor fits in 32 bits: two hardware divisions
        moveq   #0,d6
        move.l  d2,d1                   ; high half first
        divul.l d5,d6:d1                ; d1 = hi / den, d6 = hi % den
        move.l  d1,d2
        move.l  d3,d1
        divu.l  d5,d6:d1                ; (rem:lo) / den -> d1 quotient, d6 remainder
        move.l  d1,d3
        move.l  d6,d7
        moveq   #0,d6
        rts
.long:  ; general case: restoring shift-subtract, 64 steps
        moveq   #0,d6
        moveq   #0,d7
        moveq   #63,d0
.step:  add.l   d3,d3                   ; rem:num <<= 1
        addx.l  d2,d2
        addx.l  d7,d7
        addx.l  d6,d6
        cmp.l   d4,d6
        bhi.s   .sub
        bcs.s   .next
        cmp.l   d5,d7
        bcs.s   .next
.sub:   sub.l   d5,d7
        subx.l  d4,d6
        addq.l  #1,d3                   ; quotient bit
.next:  dbra    d0,.step
        rts

; Load the two 64-bit arguments (after movem of d2-d7, 24 bytes) into d2:d3
; and d4:d5.
ARGS64  macro
        movem.l d2-d7,-(sp)
        move.l  28(sp),d2
        move.l  32(sp),d3
        move.l  36(sp),d4
        move.l  40(sp),d5
        endm

__udivdi3:
        ARGS64
        bsr     udivmod64
        move.l  d2,d0
        move.l  d3,d1
        movem.l (sp)+,d2-d7
        rts

__umoddi3:
        ARGS64
        bsr     udivmod64
        move.l  d6,d0
        move.l  d7,d1
        movem.l (sp)+,d2-d7
        rts

; signed: divide magnitudes; the quotient is negative if the signs differ, the
; remainder takes the sign of the dividend
__divdi3:
        ARGS64
        moveq   #0,d0
        move.l  d2,d1
        eor.l   d4,d1                   ; sign of the quotient
        move.l  d1,a1
        bsr.s   abs_args
        bsr     udivmod64
        move.l  a1,d1
        bpl.s   .pos
        neg.l   d3
        negx.l  d2
.pos:   move.l  d2,d0
        move.l  d3,d1
        movem.l (sp)+,d2-d7
        rts

__moddi3:
        ARGS64
        move.l  d2,a1                   ; sign of the dividend
        bsr.s   abs_args
        bsr     udivmod64
        move.l  a1,d1
        bpl.s   .pos
        neg.l   d7
        negx.l  d6
.pos:   move.l  d6,d0
        move.l  d7,d1
        movem.l (sp)+,d2-d7
        rts

; make d2:d3 and d4:d5 non-negative
abs_args:
        tst.l   d2
        bpl.s   .a
        neg.l   d3
        negx.l  d2
.a:     tst.l   d4
        bpl.s   .b
        neg.l   d5
        negx.l  d4
.b:     rts

; ---- 64-bit shifts: (hi:lo, count) --------------------------------------------
__ashldi3:
        move.l  d2,-(sp)
        move.l  8(sp),d0                ; hi
        move.l  12(sp),d1               ; lo
        move.l  16(sp),d2               ; count
        and.l   #63,d2
        beq.s   .done
        cmp.l   #32,d2
        blo.s   .small
        sub.l   #32,d2
        move.l  d1,d0
        lsl.l   d2,d0
        moveq   #0,d1
        bra.s   .done
.small: lsl.l   d2,d0                   ; hi <<= n
        move.l  d1,a0
        neg.l   d2
        add.l   #32,d2
        lsr.l   d2,d1                   ; lo >> (32 - n)
        or.l    d1,d0
        move.l  a0,d1
        neg.l   d2
        add.l   #32,d2
        lsl.l   d2,d1
.done:  move.l  (sp)+,d2
        rts

__lshrdi3:
        move.l  d2,-(sp)
        move.l  8(sp),d0
        move.l  12(sp),d1
        move.l  16(sp),d2
        and.l   #63,d2
        beq.s   .done
        cmp.l   #32,d2
        blo.s   .small
        sub.l   #32,d2
        move.l  d0,d1
        lsr.l   d2,d1
        moveq   #0,d0
        bra.s   .done
.small: lsr.l   d2,d1                   ; lo >>= n
        move.l  d0,a0
        neg.l   d2
        add.l   #32,d2
        lsl.l   d2,d0                   ; hi << (32 - n)
        or.l    d0,d1
        move.l  a0,d0
        neg.l   d2
        add.l   #32,d2
        lsr.l   d2,d0
.done:  move.l  (sp)+,d2
        rts

__ashrdi3:
        move.l  d2,-(sp)
        move.l  8(sp),d0
        move.l  12(sp),d1
        move.l  16(sp),d2
        and.l   #63,d2
        beq.s   .done
        cmp.l   #32,d2
        blo.s   .small
        sub.l   #32,d2
        move.l  d0,d1
        asr.l   d2,d1
        add.l   d0,d0                   ; d0 = sign fill
        subx.l  d0,d0
        bra.s   .done
.small: lsr.l   d2,d1
        move.l  d0,a0
        neg.l   d2
        add.l   #32,d2
        lsl.l   d2,d0
        or.l    d0,d1
        move.l  a0,d0
        neg.l   d2
        add.l   #32,d2
        asr.l   d2,d0
.done:  move.l  (sp)+,d2
        rts

; ---- comparisons: 0 if a < b, 1 if equal, 2 if a > b -------------------------
__cmpdi2:
        move.l  4(sp),d0
        cmp.l   12(sp),d0
        blt.s   cmp_lt
        bgt.s   cmp_gt
        bra.s   cmp_low
__ucmpdi2:
        move.l  4(sp),d0
        cmp.l   12(sp),d0
        bcs.s   cmp_lt
        bhi.s   cmp_gt
cmp_low:
        move.l  8(sp),d0
        cmp.l   16(sp),d0
        bcs.s   cmp_lt
        bhi.s   cmp_gt
        moveq   #1,d0
        rts
cmp_lt: moveq   #0,d0
        rts
cmp_gt: moveq   #2,d0
        rts

; ---- 128-bit division --------------------------------------------------------
; u128 op(u128 a, u128 b): the result goes through a hidden pointer passed
; first, which the callee pops (rtd #4). The work is done by azrt_divmod128
; (lib/azrt, Rust).
        xdef    __udivti3
        xdef    __umodti3
        xdef    __divti3
        xdef    __modti3
        xref    azrt_divmod128

__udivti3:
        moveq   #0,d0                   ; bit0: want remainder, bit1: signed
        bra.s   ti_common
__umodti3:
        moveq   #1,d0
        bra.s   ti_common
__divti3:
        moveq   #2,d0
        bra.s   ti_common
__modti3:
        moveq   #3,d0
ti_common:
        link    a6,#-32                 ; -32(a6) quotient, -16(a6) remainder
        move.l  d0,-(sp)
        lsr.l   #1,d0
        move.l  d0,-(sp)                ; signed
        pea     -16(a6)
        pea     -32(a6)
        pea     28(a6)                  ; b
        pea     12(a6)                  ; a
        jsr     azrt_divmod128
        lea     20(sp),sp
        move.l  (sp)+,d0
        lea     -32(a6),a0
        btst    #0,d0
        beq.s   .copy
        lea     -16(a6),a0
.copy:  move.l  8(a6),a1                ; result pointer
        move.l  (a0)+,(a1)+
        move.l  (a0)+,(a1)+
        move.l  (a0)+,(a1)+
        move.l  (a0)+,(a1)+
        move.l  8(a6),d0
        move.l  d0,a0
        unlk    a6
        rtd     #4
