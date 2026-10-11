;=============================================================================
; setjmp / longjmp
;
; jmp_buf layout (longs): 0 return address, 1 sp, 2-7 d2-d7, 8-12 a2-a6,
; then fp2-fp7 (12 bytes each).
;=============================================================================

        xdef    _setjmp
        xdef    _longjmp
        xdef    __setjmp
        xdef    __longjmp
        xdef    _sigsetjmp
        xdef    _siglongjmp

        section .text
_sigsetjmp:
__setjmp:
_setjmp:
        move.l  4(sp),a0                ; jmp_buf
        move.l  (sp),(a0)+              ; return address
        lea     4(sp),a1
        move.l  a1,(a0)+                ; caller's sp after return
        movem.l d2-d7/a2-a6,(a0)
        lea     44(a0),a0
        fmovem.x fp2-fp7,(a0)
        moveq   #0,d0
        rts

_siglongjmp:
__longjmp:
_longjmp:
        move.l  4(sp),a0                ; jmp_buf
        move.l  8(sp),d0                ; value
        bne     .nz
        moveq   #1,d0
.nz:    lea     52(a0),a1
        fmovem.x (a1),fp2-fp7
        lea     8(a0),a1
        movem.l (a1),d2-d7/a2-a6
        move.l  4(a0),sp
        move.l  (a0),a1
        jmp     (a1)
