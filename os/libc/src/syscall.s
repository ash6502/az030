;=============================================================================
; long __syscall(long nr, long a1, long a2, long a3, long a4, long a5)
;
; trap #0 with the number in d0 and arguments in d1-d5. Returns the kernel's
; result: a negated errno value on failure.
;=============================================================================

        xdef    ___syscall
        xdef    ___sigreturn

        section .text
___syscall:
        movem.l d2-d5,-(sp)
        movem.l 20(sp),d0-d5
        trap    #0
        movem.l (sp)+,d2-d5
        rts

; Signal handlers return here (installed as the sa_restorer).
___sigreturn:
        moveq   #53,d0
        trap    #0
        illegal
