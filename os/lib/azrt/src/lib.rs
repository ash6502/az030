//! Runtime support written in Rust (compiled by the patched llc), called from the
//! assembly entry points in `lib/rtasm`. Kept free of 128-bit arithmetic so it cannot
//! recurse into the very helpers it implements.

#![no_std]

/// 128-bit numbers as four 32-bit limbs, most significant first (memory order).
type Limbs = [u32; 4];

fn is_zero(x: &Limbs) -> bool {
    x.iter().all(|&l| l == 0)
}

fn ge(a: &Limbs, b: &Limbs) -> bool {
    for i in 0..4 {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn sub(a: &mut Limbs, b: &Limbs) {
    let mut borrow = 0u64;
    for i in (0..4).rev() {
        let d = (a[i] as u64).wrapping_sub(b[i] as u64).wrapping_sub(borrow);
        a[i] = d as u32;
        borrow = (d >> 63) & 1;
    }
}

/// Shift left by one, shifting `bit` in at the bottom; returns the bit shifted out.
fn shl1(a: &mut Limbs, bit: u32) -> u32 {
    let out = a[0] >> 31;
    for i in 0..3 {
        a[i] = (a[i] << 1) | (a[i + 1] >> 31);
    }
    a[3] = (a[3] << 1) | bit;
    out
}

fn neg(a: &mut Limbs) {
    let mut carry = 1u64;
    for i in (0..4).rev() {
        let s = (!a[i]) as u64 + carry;
        a[i] = s as u32;
        carry = s >> 32;
    }
}

fn udivmod(n: &Limbs, d: &Limbs) -> (Limbs, Limbs) {
    if is_zero(d) {
        // Rust checks for division by zero before calling; be defined anyway
        return ([u32::MAX; 4], *n);
    }
    let mut q = *n;
    let mut r = [0u32; 4];
    for _ in 0..128 {
        let top = shl1(&mut q, 0);
        shl1(&mut r, top);
        if ge(&r, d) {
            sub(&mut r, d);
            q[3] |= 1;
        }
    }
    (q, r)
}

/// Division of 128-bit integers: `n / d` into `q`, `n % d` into `r`.
///
/// # Safety
/// All pointers must be valid for 16 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn azrt_divmod128(n: *const Limbs, d: *const Limbs, q: *mut Limbs, r: *mut Limbs, signed: u32) {
    let (mut a, mut b) = unsafe { (*n, *d) };
    let (mut neg_q, mut neg_r) = (false, false);
    if signed != 0 {
        if a[0] >> 31 == 1 {
            neg(&mut a);
            neg_q = true;
            neg_r = true;
        }
        if b[0] >> 31 == 1 {
            neg(&mut b);
            neg_q = !neg_q;
        }
    }
    let (mut qq, mut rr) = udivmod(&a, &b);
    if neg_q {
        neg(&mut qq);
    }
    if neg_r {
        neg(&mut rr);
    }
    unsafe {
        *q = qq;
        *r = rr;
    }
}
