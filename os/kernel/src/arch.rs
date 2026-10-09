//! Bindings to the assembly in `asm/entry.s` and 68030 constants.

unsafe extern "C" {
    static vector_table: [u32; 256];
    fn set_vbr(addr: u32);
    fn irq_disable() -> u32;
    fn irq_enable();
    fn irq_restore(sr: u32);
    fn read_sr() -> u32;
    fn cpu_idle();
    fn halt_forever() -> !;
    fn machine_restart() -> !;
    fn probe_read32(addr: u32, value: *mut u32) -> u32;
    fn fpu_present() -> u32;
    fn fpu_enable_switching();
    pub fn switch_context(old_ksp: *mut u32, new_ksp: u32, old_fpu: *mut u8, new_fpu: *const u8);
    fn mmu_set_tt(tt0: u32, tt1: u32);
    fn mmu_set_crp(root: u32);
    fn mmu_set_tc(tc: u32);
    fn mmu_flush();
    pub fn ret_to_user();
}

/// Size of a per-process FPU save area (see switch_context).
pub const FPU_AREA: usize = 324;

static mut HAVE_FPU: bool = false;

pub fn init() {
    unsafe { set_vbr(core::ptr::addr_of!(vector_table) as u32) };
}

pub fn init_fpu() {
    unsafe {
        if fpu_present() != 0 {
            HAVE_FPU = true;
            fpu_enable_switching();
            kprintln!("fpu: MC68882");
        } else {
            kprintln!("fpu: none");
        }
    }
}

pub fn have_fpu() -> bool {
    unsafe { HAVE_FPU }
}

/// Mask interrupts; returns the previous status register for `irq_restore`.
#[inline]
pub fn irq_save() -> u32 {
    unsafe { irq_disable() }
}

#[inline]
pub fn irq_restore_sr(sr: u32) {
    unsafe { irq_restore(sr) }
}

pub fn irq_on() {
    unsafe { irq_enable() }
}

pub fn irq_off() {
    unsafe {
        irq_disable();
    }
}

pub fn sr() -> u32 {
    unsafe { read_sr() }
}

/// Wait for an interrupt (with interrupts enabled), then mask them again.
pub fn idle() {
    unsafe {
        cpu_idle();
        irq_disable();
    }
}

pub fn halt() -> ! {
    unsafe { halt_forever() }
}

pub fn restart() -> ! {
    unsafe { machine_restart() }
}

/// Read a 32-bit register, or None if the access bus-errors (device absent).
pub fn probe(addr: u32) -> Option<u32> {
    let mut v = 0u32;
    if unsafe { probe_read32(addr, &mut v) } != 0 { Some(v) } else { None }
}

pub fn mmu_tt(tt0: u32, tt1: u32) {
    unsafe { mmu_set_tt(tt0, tt1) }
}

pub fn mmu_crp(root: u32) {
    unsafe { mmu_set_crp(root) }
}

pub fn mmu_tc(tc: u32) {
    unsafe { mmu_set_tc(tc) }
}

pub fn mmu_flush_all() {
    unsafe { mmu_flush() }
}

/// 32-bit register access for the peripheral block (32-bit accesses only).
#[inline]
pub fn rd(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline]
pub fn wr(addr: u32, v: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, v) }
}
