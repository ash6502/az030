;=============================================================================
; Floating-point "soft-float" entry points (compiler-rt names), implemented
; with the MC68882. LLVM's m68k backend passes f32/f64 as integer bits: f32 in
; one 4-byte stack slot / d0, f64 in two slots (high first) / d0:d1.
;
; Each routine first sets the FPCR rounding precision to the result format
; (single or double, round to nearest), so results are correctly rounded IEEE
; values rather than extended results rounded a second time.
;
; User programs only: the kernel does not save FPU state for itself.
;=============================================================================

        xdef    __addsf3,__subsf3,__mulsf3,__divsf3
        xdef    __adddf3,__subdf3,__muldf3,__divdf3
        xdef    __eqsf2,__nesf2,__ltsf2,__lesf2,__cmpsf2,__gtsf2,__gesf2,__unordsf2
        xdef    __eqdf2,__nedf2,__ltdf2,__ledf2,__cmpdf2,__gtdf2,__gedf2,__unorddf2
        xdef    __extendsfdf2,__truncdfsf2
        xdef    __fixsfsi,__fixdfsi,__fixunssfsi,__fixunsdfsi
        xdef    __floatsisf,__floatsidf,__floatunsisf,__floatunsidf
        xdef    __fixsfdi,__fixdfdi,__fixunssfdi,__fixunsdfdi
        xdef    __floatdisf,__floatdidf,__floatundisf,__floatundidf
        xdef    __negsf2,__negdf2
        xdef    __powisf2,__powidf2

        section .text

FPCR_S  equ     $40                     ; round to single, nearest
FPCR_D  equ     $80                     ; round to double, nearest

; return fp0 as f64 in d0:d1
RET_D   macro
        fmove.d fp0,-(sp)
        move.l  (sp)+,d0
        move.l  (sp)+,d1
        rts
        endm

; ---- arithmetic -------------------------------------------------------------
__addsf3:
        fmove.l #FPCR_S,fpcr
        fmove.s 4(sp),fp0
        fadd.s  8(sp),fp0
        fmove.s fp0,d0
        rts
__subsf3:
        fmove.l #FPCR_S,fpcr
        fmove.s 4(sp),fp0
        fsub.s  8(sp),fp0
        fmove.s fp0,d0
        rts
__mulsf3:
        fmove.l #FPCR_S,fpcr
        fmove.s 4(sp),fp0
        fmul.s  8(sp),fp0
        fmove.s fp0,d0
        rts
__divsf3:
        fmove.l #FPCR_S,fpcr
        fmove.s 4(sp),fp0
        fdiv.s  8(sp),fp0
        fmove.s fp0,d0
        rts
__adddf3:
        fmove.l #FPCR_D,fpcr
        fmove.d 4(sp),fp0
        fadd.d  12(sp),fp0
        RET_D
__subdf3:
        fmove.l #FPCR_D,fpcr
        fmove.d 4(sp),fp0
        fsub.d  12(sp),fp0
        RET_D
__muldf3:
        fmove.l #FPCR_D,fpcr
        fmove.d 4(sp),fp0
        fmul.d  12(sp),fp0
        RET_D
__divdf3:
        fmove.l #FPCR_D,fpcr
        fmove.d 4(sp),fp0
        fdiv.d  12(sp),fp0
        RET_D
__negsf2:
        move.l  4(sp),d0
        bchg    #31,d0
        rts
__negdf2:
        move.l  4(sp),d0
        move.l  8(sp),d1
        bchg    #31,d0
        rts

; ---- comparisons ----------------------------------------------------------------
; eq/ne/lt/le/cmp: -1 less, 0 equal, 1 greater or unordered
; gt/ge:           -1 less or unordered, 0 equal, 1 greater
__eqsf2:
__nesf2:
__ltsf2:
__lesf2:
__cmpsf2:
        fmove.s 4(sp),fp0
        fcmp.s  8(sp),fp0
        bra.s   cmp_unord_gt
__gtsf2:
__gesf2:
        fmove.s 4(sp),fp0
        fcmp.s  8(sp),fp0
        bra.s   cmp_unord_lt
__eqdf2:
__nedf2:
__ltdf2:
__ledf2:
__cmpdf2:
        fmove.d 4(sp),fp0
        fcmp.d  12(sp),fp0
cmp_unord_gt:
        fbun    cmp_gt
        bra.s   cmp_ordered
__gtdf2:
__gedf2:
        fmove.d 4(sp),fp0
        fcmp.d  12(sp),fp0
cmp_unord_lt:
        fbun    cmp_lt
cmp_ordered:
        fbeq    cmp_eq
        fblt    cmp_lt
cmp_gt: moveq   #1,d0
        rts
cmp_eq: moveq   #0,d0
        rts
cmp_lt: moveq   #-1,d0
        rts

__unordsf2:
        fmove.s 4(sp),fp0
        fcmp.s  8(sp),fp0
        bra.s   unord
__unorddf2:
        fmove.d 4(sp),fp0
        fcmp.d  12(sp),fp0
unord:  fbun    .yes
        moveq   #0,d0
        rts
.yes:   moveq   #1,d0
        rts

; ---- conversions between formats ----------------------------------------------
__extendsfdf2:
        fmove.s 4(sp),fp0
        RET_D
__truncdfsf2:
        fmove.d 4(sp),fp0
        fmove.s fp0,d0
        rts

; ---- float -> 32-bit integer (truncating) -------------------------------------
__fixsfsi:
        fintrz.s 4(sp),fp0
        fmove.l fp0,d0
        rts
__fixdfsi:
        fintrz.d 4(sp),fp0
        fmove.l fp0,d0
        rts
__fixunssfsi:
        fintrz.s 4(sp),fp0
        bra.s   fix_u32
__fixunsdfsi:
        fintrz.d 4(sp),fp0
; fp0 (integral, >= 0) -> d0 as unsigned
fix_u32:
        ftst.x  fp0
        fblt    .zero
        fcmp.l  #$7FFFFFFF,fp0
        fbgt    .big
        fmove.l fp0,d0
        rts
.big:   fsub.d  #2147483648.0,fp0
        fmove.l fp0,d0
        bset    #31,d0
        rts
.zero:  moveq   #0,d0
        rts

; ---- 32-bit integer -> float ------------------------------------------------------
__floatsisf:
        fmove.l 4(sp),fp0
        fmove.s fp0,d0
        rts
__floatsidf:
        fmove.l 4(sp),fp0
        RET_D
__floatunsisf:
        bsr.s   u32_fp0
        fmove.s fp0,d0
        rts
__floatunsidf:
        bsr.s   u32_fp0
        RET_D
; unsigned 4(sp) of the caller -> fp0
u32_fp0:
        fmove.l 8(sp),fp0
        tst.l   8(sp)
        bpl.s   .ok
        fadd.d  #4294967296.0,fp0
.ok:    rts

; ---- 64-bit integer -> float --------------------------------------------------------
; value = hi * 2^32 + lo (lo unsigned); exact in extended precision
__floatdisf:
        bsr.s   i64_fp0
        fmove.s fp0,d0
        rts
__floatdidf:
        bsr.s   i64_fp0
        RET_D
__floatundisf:
        bsr.s   u64_fp0
        fmove.s fp0,d0
        rts
__floatundidf:
        bsr.s   u64_fp0
        RET_D
i64_fp0:
        fmove.l 8(sp),fp0               ; signed high word
        bra.s   lo_part
u64_fp0:
        fmove.l 8(sp),fp0
        tst.l   8(sp)
        bpl.s   lo_part
        fadd.d  #4294967296.0,fp0
lo_part:
        fmul.d  #4294967296.0,fp0
        fmove.l 12(sp),fp1
        tst.l   12(sp)
        bpl.s   .ok
        fadd.d  #4294967296.0,fp1
.ok:    fadd.x  fp1,fp0
        rts

; ---- float -> 64-bit integer (truncating) ---------------------------------------
__fixsfdi:
        fintrz.s 4(sp),fp0
        bra.s   fix_i64
__fixdfdi:
        fintrz.d 4(sp),fp0
fix_i64:
        ftst.x  fp0
        fbge    fix_u64
        fneg.x  fp0
        bsr.s   fix_u64
        neg.l   d1
        negx.l  d0
        rts
__fixunssfdi:
        fintrz.s 4(sp),fp0
        bra.s   fix_u64
__fixunsdfdi:
        fintrz.d 4(sp),fp0
; fp0 (integral, >= 0) -> d0:d1 as unsigned 64-bit
fix_u64:
        ftst.x  fp0
        fblt    .zero
        fmove.x fp0,fp1
        fdiv.d  #4294967296.0,fp1
        fintrz.x fp1                    ; high word
        fmove.x fp1,fp2
        fmul.d  #4294967296.0,fp2
        fsub.x  fp2,fp0                 ; low word, exactly
        fmove.x fp1,fp2
        fcmp.l  #$7FFFFFFF,fp2
        fble    .hi_small
        fsub.d  #2147483648.0,fp2
        fmove.l fp2,d0
        bset    #31,d0
        bra.s   .lo
.hi_small:
        fmove.l fp2,d0
.lo:    fcmp.l  #$7FFFFFFF,fp0
        fble    .lo_small
        fsub.d  #2147483648.0,fp0
        fmove.l fp0,d1
        bset    #31,d1
        rts
.lo_small:
        fmove.l fp0,d1
        rts
.zero:  moveq   #0,d0
        moveq   #0,d1
        rts

; ---- x ** n for integer n -------------------------------------------------------------
__powisf2:
        fmove.s 4(sp),fp1
        move.l  8(sp),d0
        bsr.s   powi
        fmove.s fp0,d0
        rts
__powidf2:
        fmove.d 4(sp),fp1
        move.l  12(sp),d0
        bsr.s   powi
        RET_D
; fp0 = fp1 ** d0 by repeated squaring
powi:
        move.l  d0,d1
        bpl.s   .pos
        neg.l   d0
.pos:   fmove.l #1,fp0
.loop:  tst.l   d0
        beq.s   .done
        btst    #0,d0
        beq.s   .sq
        fmul.x  fp1,fp0
.sq:    fmul.x  fp1,fp1
        lsr.l   #1,d0
        bra.s   .loop
.done:  tst.l   d1
        bpl.s   .ret
        fmove.l #1,fp1
        fdiv.x  fp0,fp1
        fmove.x fp1,fp0
.ret:   rts
