;=============================================================================
; Math functions implemented with 68881/68882 instructions.
; double f(double x): x at 4(sp); the result is returned in fp0.
;=============================================================================

        section .text

UNARY   macro
        xdef    _\1
_\1:    \2.d    4(sp),fp0
        rts
        endm

        UNARY   sqrt,fsqrt
        UNARY   sin,fsin
        UNARY   cos,fcos
        UNARY   tan,ftan
        UNARY   asin,fasin
        UNARY   acos,facos
        UNARY   atan,fatan
        UNARY   sinh,fsinh
        UNARY   cosh,fcosh
        UNARY   tanh,ftanh
        UNARY   atanh,fatanh
        UNARY   exp,fetox
        UNARY   exp2,ftwotox
        UNARY   expm1,fetoxm1
        UNARY   log,flogn
        UNARY   log10,flog10
        UNARY   log2,flog2
        UNARY   log1p,flognp1
        UNARY   fabs,fabs
        UNARY   trunc,fintrz
        UNARY   rint,fint
        UNARY   nearbyint,fint

; float versions (argument passed as a double after promotion is not
; guaranteed: prototyped float arguments arrive as 4-byte floats)
FUNARY  macro
        xdef    _\1f
_\1f:   \2.s    4(sp),fp0
        rts
        endm

        FUNARY  sqrt,fsqrt
        FUNARY  sin,fsin
        FUNARY  cos,fcos
        FUNARY  tan,ftan
        FUNARY  atan,fatan
        FUNARY  exp,fetox
        FUNARY  log,flogn
        FUNARY  fabs,fabs

        xdef    _fmod
_fmod:  fmove.d 4(sp),fp0
        fmod.d  12(sp),fp0
        rts

        xdef    _remainder
_remainder:
        fmove.d 4(sp),fp0
        frem.d  12(sp),fp0
        rts

; double ldexp(double x, int e)
        xdef    _ldexp
        xdef    _scalbn
_scalbn:
_ldexp: fmove.d 4(sp),fp0
        fscale.l 12(sp),fp0
        rts

; double frexp(double x, int *e): x = m * 2^e with 0.5 <= |m| < 1
        xdef    _frexp
_frexp: fmove.d 4(sp),fp0
        move.l  12(sp),a0
        ftst.x  fp0
        fbeq    .zero
        fgetexp.x fp0,fp1
        fmove.l fp1,d0
        addq.l  #1,d0
        move.l  d0,(a0)
        fgetman.x fp0,fp0               ; 1 <= |m| < 2
        fscale.l #-1,fp0
        rts
.zero:  clr.l   (a0)
        rts

; double atan2(double y, double x)
        xdef    _atan2
_atan2: fmove.d 4(sp),fp0               ; y
        fmove.d 12(sp),fp1              ; x
        ftst.x  fp1
        fbeq    .xzero
        fdiv.x  fp1,fp0
        fatan.x fp0
        ftst.x  fp1
        fbgt    .done                   ; x > 0
        fmove.d 4(sp),fp1
        ftst.x  fp1
        fblt    .neg
        fadd.d  .pi,fp0                 ; x < 0, y >= 0
        rts
.neg:   fsub.d  .pi,fp0                 ; x < 0, y < 0
        rts
.xzero: ftst.x  fp0
        fbeq    .zr
        fblt    .ny
        fmove.d .pi2,fp0
        rts
.ny:    fmove.d .pi2,fp0
        fneg.x  fp0
        rts
.zr:    fmove.x fp0,fp0
.done:  rts

        section .rodata
        even
.pi:    dc.l    $400921FB,$54442D18
.pi2:   dc.l    $3FF921FB,$54442D18

        section .text
; FPCR / FPSR access for fenv.h
        xdef    ___get_fpcr
___get_fpcr:
        fmove.l fpcr,d0
        rts
        xdef    ___set_fpcr
___set_fpcr:
        fmove.l 4(sp),fpcr
        rts
