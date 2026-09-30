| -----------------------------------------------------------------------------
| boot.S -- minimal 68030 boot: prints a banner on the UART, then echoes input.
| Build: make -C fw      (needs m68k-elf-binutils)
| All peripheral registers are 32-bit access only.
| -----------------------------------------------------------------------------
        .equ    UART_DATA,  0xFE000000
        .equ    UART_STAT,  0xFE000004
        .equ    SYS_BOOT,   0xFE001000
        .equ    SYS_LEDS,   0xFE001008

        .section .vectors, "a"
        .global _vectors
_vectors:
        .long   0x00008000              | 0: initial SSP (top of 32 KB RAM)
        .long   _start                  | 1: initial PC  (runs from high ROM alias)
        .rept   254
        .long   _fault                  | 2..255: everything else
        .endr

        .text
        .global _start
_start:
        move.w  #0x2700,%sr             | supervisor, interrupts masked
        lea     _vectors,%a0
        movec   %a0,%vbr                | vectors stay in ROM, RAM is all ours
        move.l  %d0,SYS_BOOT            | any write: drop ROM overlay at 0x0 -> RAM
        moveq   #1,%d0
        move.l  %d0,SYS_LEDS

        lea     msg,%a0
1:      move.b  (%a0)+,%d0
        beq     2f
        bsr     putc
        bra     1b

2:      bsr     getc                    | echo loop
        bsr     putc
        bra     2b

putc:   move.l  UART_STAT,%d1
        btst    #0,%d1                  | tx_ready?
        beq     putc
        andi.l  #0xFF,%d0
        move.l  %d0,UART_DATA
        rts

getc:   move.l  UART_STAT,%d1
        btst    #1,%d1                  | rx_valid?
        beq     getc
        move.l  UART_DATA,%d0
        rts

_fault: moveq   #0xF,%d0
        move.l  %d0,SYS_LEDS            | light all LEDs on any exception
        bra     _fault

        .section .rodata
msg:    .asciz  "\r\nretro030 alive.\r\n"
