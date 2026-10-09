;=============================================================================
; az030 kernel: low-level entry points (assembled with azas, linked with azld)
;
; Calling convention shared with Rust (LLVM m68k, C ABI): arguments on the
; stack, results in d0, d0/d1/a0/a1 are scratch, d2-d7/a2-a6 are preserved.
; Functions here that return an address return it in both d0 and a0.
;=============================================================================

        xdef    _start
        xdef    vector_table
        xdef    exc_return
        xdef    ret_to_user
        xdef    switch_context
        xdef    cpu_idle
        xdef    irq_disable
        xdef    irq_enable
        xdef    irq_restore
        xdef    read_sr
        xdef    set_vbr
        xdef    mmu_set_tt
        xdef    mmu_set_crp
        xdef    mmu_set_tc
        xdef    mmu_flush
        xdef    probe_read32
        xdef    fpu_present
        xdef    machine_restart
        xdef    halt_forever

        xref    kmain
        xref    trap_handler
        xref    __bss_start
        xref    __bss_end

BOOT_STACK      equ     16384

        section .text

;-----------------------------------------------------------------------------
; Entry from the bootloader: a0 = boot info, d0 = SCSI ID, d1 = RAM size.
;-----------------------------------------------------------------------------
_start:
        move.w  #$2700,sr
        move.l  a0,d7                           ; keep boot info across bss clear
        lea     __bss_start,a1
        lea     __bss_end,a2
.clr:   cmp.l   a2,a1
        bhs.s   .clred
        clr.b   (a1)+
        bra.s   .clr
.clred: lea     boot_stack+BOOT_STACK,sp
        move.l  d7,-(sp)
        jsr     kmain                           ; kmain(bootinfo) never returns
halt_forever:
        move.w  #$2700,sr
.h:     stop    #$2700
        bra.s   .h

;-----------------------------------------------------------------------------
; Exceptions, interrupts and system calls all come through here. The trap frame
; (see trap.rs) is built on the kernel stack:
;   0   usp
;   4   d0-d7
;   36  a0-a6
;   64  CPU exception frame (sr, pc, format/vector, ...)
;-----------------------------------------------------------------------------
exc_common:
        tst.l   probe_sp                        ; bus error while probing?
        bne.s   probe_fault
exc_save:
        movem.l d0-d7/a0-a6,-(sp)
        move.l  usp,a0
        move.l  a0,-(sp)
        move.l  sp,-(sp)
        jsr     trap_handler
        addq.l  #4,sp
exc_return:
ret_to_user:
        move.l  (sp)+,a0
        move.l  a0,usp
        movem.l (sp)+,d0-d7/a0-a6
        rte

; A bus error during probe_read32: abandon the frame and fail the probe.
probe_fault:
        move.w  6(sp),d0
        and.w   #$0FFF,d0
        cmp.w   #8,d0                           ; vector 2 (bus error) * 4
        bne.s   exc_save
        move.l  probe_sp,sp
        clr.l   probe_sp
        move.w  d1,sr                           ; saved by probe_read32
        moveq   #0,d0                           ; failure
        rts

;-----------------------------------------------------------------------------
; u32 probe_read32(u32 addr, u32 *value): 1 if the read worked, 0 on bus error.
;-----------------------------------------------------------------------------
probe_read32:
        move.l  4(sp),a0
        move.l  8(sp),a1
        move.w  sr,d1                           ; the fault path restores it
        or.w    #$0700,sr
        move.l  sp,probe_sp                     ; ...and returns from here
        move.l  (a0),d0
        nop                                     ; let a bus error land here
        clr.l   probe_sp
        move.l  d0,(a1)
        move.w  d1,sr
        moveq   #1,d0
        rts

;-----------------------------------------------------------------------------
; u32 fpu_present(void): 1 if an FPU answers.
;-----------------------------------------------------------------------------
fpu_present:
        move.l  vector_table+11*4,-(sp)         ; line F vector
        move.l  #.nofpu,vector_table+11*4
        move.l  sp,fpu_probe_sp
        fmove.l fpcr,d0
        move.l  (sp)+,vector_table+11*4
        moveq   #1,d0
        rts
.nofpu: move.l  fpu_probe_sp,sp
        move.l  (sp)+,vector_table+11*4
        moveq   #0,d0
        rts

;-----------------------------------------------------------------------------
; void switch_context(u32 *old_ksp, u32 new_ksp, u8 *old_fpu, u8 *new_fpu)
;
; Saves the callee-saved registers and FPU state of the current context, then
; resumes the context whose kernel stack pointer is new_ksp. FPU save areas are
; FPU_AREA bytes: fsave frame (up to 216 bytes), fp0-fp7 at 216, control
; registers at 312. A zeroed area is a null frame (FPU never used).
;-----------------------------------------------------------------------------
FPU_REGS        equ     216
FPU_CTRL        equ     312

switch_context:
        movem.l d2-d7/a2-a6,-(sp)               ; 44 bytes
        move.l  44+4(sp),a2                     ; old_ksp
        move.l  44+8(sp),d2                     ; new_ksp
        move.l  44+12(sp),a3                    ; old_fpu
        move.l  44+16(sp),a4                    ; new_fpu
        tst.b   fpu_ok
        beq.s   .nofpu1
        move.l  a3,d0
        beq.s   .nofpu1
        fsave   (a3)
        tst.b   (a3)
        beq.s   .nofpu1
        fmovem.x fp0-fp7,FPU_REGS(a3)
        fmovem.l fpcr/fpsr/fpiar,FPU_CTRL(a3)
.nofpu1:
        move.l  sp,(a2)
        move.l  d2,sp
        tst.b   fpu_ok
        beq.s   .nofpu2
        move.l  a4,d0
        beq.s   .nofpu2
        tst.b   (a4)
        beq.s   .null
        fmovem.l FPU_CTRL(a4),fpcr/fpsr/fpiar
        fmovem.x FPU_REGS(a4),fp0-fp7
.null:  frestore (a4)
.nofpu2:
        movem.l (sp)+,d2-d7/a2-a6
        rts

        xdef    fpu_enable_switching
fpu_enable_switching:
        st      fpu_ok
        rts

;-----------------------------------------------------------------------------
; Idle until an interrupt arrives. Returns with interrupts enabled.
;-----------------------------------------------------------------------------
cpu_idle:
        stop    #$2000
        rts

; u32 irq_disable(void): mask all interrupts, return the previous SR.
irq_disable:
        moveq   #0,d0
        move.w  sr,d0
        or.w    #$0700,sr
        rts

; void irq_enable(void)
irq_enable:
        and.w   #$F8FF,sr
        rts

; void irq_restore(u32 sr)
irq_restore:
        move.w  6(sp),sr
        rts

; u32 read_sr(void)
read_sr:
        moveq   #0,d0
        move.w  sr,d0
        rts

; void set_vbr(u32 addr)
set_vbr:
        move.l  4(sp),d0
        movec   d0,vbr
        rts

;-----------------------------------------------------------------------------
; PMMU control.
;-----------------------------------------------------------------------------
; void mmu_set_tt(u32 tt0, u32 tt1)
mmu_set_tt:
        pmove   4(sp),tt0
        pmove   8(sp),tt1
        rts

; void mmu_set_crp(u32 root_table): load a CRP for a 4-byte-descriptor table
mmu_set_crp:
        move.l  4(sp),d0
        move.l  d0,-(sp)
        move.l  #$7FFF0002,-(sp)                ; no limit, DT = valid 4-byte
        pmove   (sp),crp
        addq.l  #8,sp
        pflusha
        rts

; void mmu_set_tc(u32 tc)
mmu_set_tc:
        pmove   4(sp),tc
        pflusha
        rts

; void mmu_flush(void)
mmu_flush:
        pflusha
        rts

;-----------------------------------------------------------------------------
; void machine_restart(void): MMU off, back to the boot ROM.
;-----------------------------------------------------------------------------
machine_restart:
        move.w  #$2700,sr
        clr.l   -(sp)
        pmove   (sp),tc
        addq.l  #4,sp
        reset
        move.l  $FFF00000,sp
        move.l  $FFF00004,a0
        jmp     (a0)

;-----------------------------------------------------------------------------
; Vector table: every vector enters exc_common.
;-----------------------------------------------------------------------------
        section .data
        cnop    0,4
vector_table:
        dc.l    0,0
        rept    254
        dc.l    exc_common
        endr

probe_sp:       dc.l    0
fpu_probe_sp:   dc.l    0
fpu_ok:         dc.b    0
        even

        section .bss
        cnop    0,4
boot_stack:     ds.b    BOOT_STACK
