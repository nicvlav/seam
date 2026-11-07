//! Bit fields of the words learn packs: a staged capture, the arm and a
//! dispatch key. Each word's layout is one list of fields, low bits first,
//! each placed just above the one before, so no two overlap.

// Audio-path code: a panic here ends a host, so indexing, unwrap, expect,
// panic and unchecked arithmetic are refused, and a site that is provably
// safe says why in an `#[expect]`.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )
)]

/// `width` bits of a `u64`, from bit `shift` up.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Field {
    shift: u32,
    width: u32,
}

impl Field {
    /// The lowest field of a word.
    pub(crate) const fn lowest(width: u32) -> Field {
        Field::at(0, width)
    }

    /// The field just above `below`.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "fields are constants, so an overflow fails the build"
    )]
    pub(crate) const fn above(below: Field, width: u32) -> Field {
        Field::at(below.shift + below.width, width)
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "fields are constants, so an overflow fails the build"
    )]
    const fn at(shift: u32, width: u32) -> Field {
        assert!(
            width > 0 && shift + width <= u64::BITS,
            "a field fits in its word"
        );
        Field { shift, width }
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "a field's width is 1 to 64, checked when it is made"
    )]
    const fn mask(self) -> u64 {
        u64::MAX >> (u64::BITS - self.width)
    }

    /// `value` in the field's place. Bits above the width are dropped; every
    /// caller's value fits.
    pub(crate) const fn put(self, value: u64) -> u64 {
        (value & self.mask()) << self.shift
    }

    /// The field's value in `word`.
    pub(crate) const fn get(self, word: u64) -> u64 {
        (word >> self.shift) & self.mask()
    }
}

#[cfg(test)]
mod tests {
    use super::Field;

    #[test]
    fn fields_stacked_on_each_other_keep_their_values_apart() {
        let low = Field::lowest(3);
        let middle = Field::above(low, 13);
        let high = Field::above(middle, 48);
        let word = low.put(0b101) | middle.put(0x1FFF) | high.put(u64::MAX);
        assert_eq!(low.get(word), 0b101);
        assert_eq!(middle.get(word), 0x1FFF);
        assert_eq!(high.get(word), (1 << 48) - 1, "cut to its width");
        assert_eq!(middle.put(1 << 13), 0, "a bit past the width is dropped");
    }
}
