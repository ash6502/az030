ENTRY(_start)
MEMORY
{
    ROM (rx)  : ORIGIN = 0xFFF00000, LENGTH = 16K
    RAM (rwx) : ORIGIN = 0x00000000, LENGTH = 32K
}
SECTIONS
{
    .vectors : { *(.vectors) } > ROM
    .text    : { *(.text*)   } > ROM
    .rodata  : { *(.rodata*) } > ROM
}
