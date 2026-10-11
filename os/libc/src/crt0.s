;=============================================================================
; C start-up code for AZ/UX.
;
; The kernel enters _start with the stack holding argc, the argv pointers, a
; NULL, the envp pointers and a NULL.
;=============================================================================

        xdef    _start
        xref    _main
        xref    _exit
        xref    _environ
        xref    ___libc_init

        section .text
_start:
        fmove.l #$80,fpcr               ; round to double precision, to nearest
        move.l  sp,a0
        move.l  (a0)+,d0                ; argc
        move.l  a0,a1                   ; argv
        lea     4(a0,d0.l*4),a0         ; envp
        move.l  a0,_environ
        sub.l   a6,a6                   ; end of the frame chain
        move.l  a0,-(sp)
        move.l  a1,-(sp)
        move.l  d0,-(sp)
        jsr     ___libc_init
        jsr     _main
        move.l  d0,-(sp)
        jsr     _exit                   ; exit(main(argc, argv, envp))
        illegal
