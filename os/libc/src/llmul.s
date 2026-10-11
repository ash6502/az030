;=============================================================================
; long long __mulll(long long a, long long b)  -- 64-bit multiply
;
; Called by compiled code with a and b on the stack (high word first);
; returns the product in d0 (high) : d1 (low).
;=============================================================================

        xdef    __mulll

        section .text
__mulll:
        movem.l d2-d4,-(sp)
        ; a.hi 16(sp), a.lo 20(sp), b.hi 24(sp), b.lo 28(sp)
        move.l  20(sp),d1               ; a.lo
        move.l  28(sp),d2               ; b.lo
        move.l  d1,d3
        mulu.l  d2,d0:d3                ; d0:d3 = a.lo * b.lo
        move.l  16(sp),d4               ; a.hi
        mulu.l  d2,d4                   ; a.hi * b.lo (low half)
        add.l   d4,d0
        move.l  24(sp),d4               ; b.hi
        mulu.l  d1,d4                   ; b.hi * a.lo (low half)
        add.l   d4,d0
        move.l  d3,d1
        movem.l (sp)+,d2-d4
        rts
