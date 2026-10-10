;=============================================================================
; Start-up code and system-call stub for Rust programs on the az030 OS.
;
; The kernel enters _start with the user stack holding argc, the argv
; pointers, a NULL, the envp pointers and a NULL (see kernel/src/exec.rs).
;=============================================================================

        xdef    _start
        xdef    rt_syscall
        xdef    rt_sigreturn
        xref    rt_entry

        section .text

_start:
        move.l  sp,a0
        move.l  (a0)+,d0                        ; argc
        move.l  a0,a1                           ; argv
        lea     4(a0,d0.l*4),a0                 ; envp (past argv's NULL)
        clr.l   -(sp)                           ; terminate frame chains
        move.l  a0,-(sp)
        move.l  a1,-(sp)
        move.l  d0,-(sp)
        jsr     rt_entry                        ; rt_entry(argc, argv, envp) never returns
        illegal

;-----------------------------------------------------------------------------
; i32 rt_syscall(u32 nr, u32 a1, u32 a2, u32 a3, u32 a4, u32 a5)
;-----------------------------------------------------------------------------
rt_syscall:
        movem.l d2-d5,-(sp)
        movem.l 20(sp),d0-d5                    ; nr, a1..a5
        trap    #0
        movem.l (sp)+,d2-d5
        rts

;-----------------------------------------------------------------------------
; Signal handlers return here (installed as sa_restorer).
;-----------------------------------------------------------------------------
rt_sigreturn:
        moveq   #53,d0                          ; SIGRETURN
        trap    #0
        illegal
