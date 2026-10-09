//! Conversion of `f64` to the 68881 96-bit extended format.

/// Sign + 15-bit exponent, 16 zero bits, 64-bit mantissa with explicit integer bit.
pub fn to_extended(f: f64) -> [u8; 12] {
    let bits = f.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (e, m): (u16, u64) = if exp == 0 && frac == 0 {
        (0, 0)
    } else if exp == 0x7FF {
        (0x7FFF, if frac == 0 { 0 } else { (1 << 63) | (frac << 11) })
    } else if exp == 0 {
        // subnormal double: normalise
        let p = 63 - frac.leading_zeros() as i32; // highest set bit
        ((p - 1074 + 16383) as u16, frac << (63 - p))
    } else {
        ((exp - 1023 + 16383) as u16, (1 << 63) | (frac << 11))
    };
    let mut out = [0u8; 12];
    out[0..2].copy_from_slice(&(sign | e).to_be_bytes());
    out[4..12].copy_from_slice(&m.to_be_bytes());
    out
}
