;=============================================================================
; Memory and string primitives for code compiled by LLVM (memcpy & co.).
; Linked into the kernel and every Rust program; they replace the versions in
; Rust's compiler_builtins.
;
; C calling convention: arguments on the stack, result in d0 (pointers also in
; a0), d0/d1/a0/a1 are scratch. The 68030 handles misaligned word and long
; accesses, so the bulk loops do not need to align.
;=============================================================================

        xdef    memcpy
        xdef    memmove
        xdef    memset
        xdef    memcmp
        xdef    bcmp
        xdef    strlen

        section .text

;-----------------------------------------------------------------------------
; void *memcpy(void *dst, const void *src, size_t n)
;-----------------------------------------------------------------------------
memcpy:
        move.l  4(sp),a0
        move.l  8(sp),a1
        move.l  12(sp),d0
copy_up:                                ; a0 = dst, a1 = src, d0 = n
        move.l  d0,d1
        lsr.l   #4,d1                   ; 16-byte blocks
        beq.s   .tail
.blk:   move.l  (a1)+,(a0)+
        move.l  (a1)+,(a0)+
        move.l  (a1)+,(a0)+
        move.l  (a1)+,(a0)+
        subq.l  #1,d1
        bne.s   .blk
.tail:  and.w   #15,d0
        bra.s   .tc
.tb:    move.b  (a1)+,(a0)+
.tc:    dbra    d0,.tb
        move.l  4(sp),d0
        move.l  d0,a0
        rts

;-----------------------------------------------------------------------------
; void *memmove(void *dst, const void *src, size_t n)
;-----------------------------------------------------------------------------
memmove:
        move.l  4(sp),a0
        move.l  8(sp),a1
        move.l  12(sp),d0
        cmp.l   a1,a0
        bls.s   copy_up                 ; dst <= src: forward copy is safe
        move.l  a1,d1
        add.l   d0,d1
        cmp.l   d1,a0
        bhs.s   copy_up                 ; no overlap
        add.l   d0,a0                   ; copy downwards from the ends
        add.l   d0,a1
        move.l  d0,d1
        lsr.l   #4,d1
        beq.s   .tail
.blk:   move.l  -(a1),-(a0)
        move.l  -(a1),-(a0)
        move.l  -(a1),-(a0)
        move.l  -(a1),-(a0)
        subq.l  #1,d1
        bne.s   .blk
.tail:  and.w   #15,d0
        bra.s   .tc
.tb:    move.b  -(a1),-(a0)
.tc:    dbra    d0,.tb
        move.l  4(sp),d0
        move.l  d0,a0
        rts

;-----------------------------------------------------------------------------
; void *memset(void *dst, int c, size_t n)
;-----------------------------------------------------------------------------
memset:
        move.l  4(sp),a0
        move.l  8(sp),d0
        move.l  12(sp),d1
        and.l   #$FF,d0                 ; replicate the byte into a long
        move.l  d0,a1
        lsl.l   #8,d0
        add.l   a1,d0
        move.l  d0,a1
        swap    d0
        clr.w   d0
        add.l   a1,d0
        move.l  d1,a1                   ; a1 = n
        lsr.l   #4,d1
        beq.s   .tail
.blk:   move.l  d0,(a0)+
        move.l  d0,(a0)+
        move.l  d0,(a0)+
        move.l  d0,(a0)+
        subq.l  #1,d1
        bne.s   .blk
.tail:  move.l  a1,d1
        and.w   #15,d1
        bra.s   .tc
.tb:    move.b  d0,(a0)+
.tc:    dbra    d1,.tb
        move.l  4(sp),d0
        move.l  d0,a0
        rts

;-----------------------------------------------------------------------------
; int memcmp(const void *a, const void *b, size_t n)
; int bcmp(const void *a, const void *b, size_t n)
;-----------------------------------------------------------------------------
memcmp:
bcmp:
        move.l  4(sp),a0
        move.l  8(sp),a1
        move.l  12(sp),d1
        beq.s   .equal
.loop:  cmpm.b  (a1)+,(a0)+
        bne.s   .differ
        subq.l  #1,d1
        bne.s   .loop
.equal: moveq   #0,d0
        rts
.differ:
        moveq   #0,d0
        moveq   #0,d1
        move.b  -(a0),d0
        move.b  -(a1),d1
        sub.l   d1,d0
        rts

;-----------------------------------------------------------------------------
; size_t strlen(const char *s)
;-----------------------------------------------------------------------------
strlen:
        move.l  4(sp),a0
        move.l  a0,d0
.loop:  tst.b   (a0)+
        bne.s   .loop
        sub.l   d0,a0
        move.l  a0,d0
        subq.l  #1,d0
        rts
