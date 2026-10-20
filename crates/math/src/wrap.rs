//! Integer conversions that wrap, as C's implicit ones do. The game's code stores a 32-bit
//! result in a 16-bit field by dropping the high bits, and loads a field unsigned when the
//! struct says `u16`; spelling it `as i16` or `as u16 as i32` hides that this is on purpose.
//! These name it, so the places the game truncates can be found, and read, at a glance.

/// Narrowing that drops the high bits (`as` between integers), and reading the low bits back
/// as a wider number.
pub trait Wrap: Copy {
    /// The low 8 bits, as a signed byte.
    fn wrap_i8(self) -> i8;
    /// The low 8 bits.
    fn wrap_u8(self) -> u8;
    /// The low 16 bits, as a signed half word.
    fn wrap_i16(self) -> i16;
    /// The low 16 bits.
    fn wrap_u16(self) -> u16;

    /// The low 8 bits, zero-extended (a load of a `u8` field).
    fn zext8(self) -> i32 {
        i32::from(self.wrap_u8())
    }

    /// The low 16 bits, zero-extended (a load of a `u16` field).
    fn zext16(self) -> i32 {
        i32::from(self.wrap_u16())
    }

    /// The low 8 bits, sign-extended (a load of an `s8` field).
    fn sext8(self) -> i32 {
        i32::from(self.wrap_i8())
    }

    /// The low 16 bits, sign-extended (a load of an `s16` field).
    fn sext16(self) -> i32 {
        i32::from(self.wrap_i16())
    }
}

macro_rules! impl_wrap {
    ($($t:ty),*) => {$(
        impl Wrap for $t {
            fn wrap_i8(self) -> i8 {
                self as i8
            }
            fn wrap_u8(self) -> u8 {
                self as u8
            }
            fn wrap_i16(self) -> i16 {
                self as i16
            }
            fn wrap_u16(self) -> u16 {
                self as u16
            }
        }
    )*};
}

impl_wrap!(i8, u8, i16, u16, i32, u32, i64, u64, isize, usize);

/// The bits of a signed word as unsigned (`(u32)value`), for registers and packed words.
pub trait Bits {
    fn bits(self) -> u32;
}

impl Bits for i32 {
    fn bits(self) -> u32 {
        self as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_drops_the_high_bits() {
        assert_eq!(0x1_2345_i32.wrap_i16(), 0x2345);
        assert_eq!(0x1_8000_i32.wrap_i16(), -0x8000);
        assert_eq!((-1_i32).wrap_u16(), 0xffff);
        assert_eq!(0x1ff_i32.wrap_i8(), -1);
        assert_eq!(0x1ff_i32.wrap_u8(), 0xff);
        assert_eq!(70_000_usize.wrap_u16(), 4464);
    }

    #[test]
    fn extending_reads_the_low_bits_back() {
        // A negative s16 field read as unsigned.
        assert_eq!((-1_i16).zext16(), 0xffff);
        assert_eq!((-2_i8).zext8(), 0xfe);
        assert_eq!(0x1_ffff_i32.sext16(), -1);
        assert_eq!(0xffff_u16.sext16(), -1);
        assert_eq!((-1_i32).bits(), u32::MAX);
        assert_eq!(i32::from(-3_i16).bits(), 0xffff_fffd);
    }
}
