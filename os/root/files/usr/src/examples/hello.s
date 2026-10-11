; hello.s -- "hello, world" in 68030 assembly for AZ/UX.
;
;   as hello.s && ld -o hello hello.o && ./hello
;
; System calls: trap #0 with the call number in d0 and arguments in d1-d5
; (see /usr/include/sys/syscall.h); the result comes back in d0.

SYS_EXIT        equ     0
SYS_WRITE       equ     3

        xdef    _start
        section .text

_start:
        moveq   #SYS_WRITE,d0
        moveq   #1,d1                   ; standard output
        move.l  #msg,d2
        move.l  #len,d3
        trap    #0
        moveq   #SYS_EXIT,d0
        moveq   #0,d1
        trap    #0

        section .rodata
msg:    dc.b    "hello, world",10
len     equ     *-msg
