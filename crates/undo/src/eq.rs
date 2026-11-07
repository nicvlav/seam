//! The equality undo decides no-ops with.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::{BuildHasher, Hash};
use std::rc::Rc;
use std::sync::Arc;

/// A reflexive equality: a value equals a copy of itself, whatever it
/// holds. Undo uses it to decide that a write changed nothing, which then
/// records nothing and keeps the redo.
///
/// `PartialEq` is not enough: under `==` a NaN differs from itself, so
/// writing a NaN over itself would record a step. Floats are equal when
/// bit-identical or `==`, so a NaN equals itself and -0 equals +0.
///
/// For your own type, [`undo_by_eq!`](crate::undo_by_eq) (a type whose `==`
/// is reflexive) and [`undo_by_fields!`](crate::undo_by_fields) (a struct,
/// field by field) write the impls.
///
/// # Examples
///
/// ```
/// use seam_undo::UndoEq;
///
/// // Hand-written for a type the macros do not fit: a gain compared in whole
/// // tenths of a decibel, so a drag that ends 0.01 dB away records no step.
/// struct Gain(f32);
///
/// impl UndoEq for Gain {
///     fn undo_eq(&self, other: &Self) -> bool {
///         (self.0 * 10.0).round() == (other.0 * 10.0).round()
///     }
/// }
///
/// assert!(Gain(-6.01).undo_eq(&Gain(-6.0)));
/// assert!(!Gain(-6.0).undo_eq(&Gain(-5.0)));
/// ```
pub trait UndoEq {
    /// Returns whether `self` and `other` are the same value for undo: the
    /// trait's example implements it.
    fn undo_eq(&self, other: &Self) -> bool;
}

macro_rules! float_undo_eq {
    ($($float:ty),*) => {$(
        impl UndoEq for $float {
            fn undo_eq(&self, other: &Self) -> bool {
                self.to_bits() == other.to_bits() || self == other
            }
        }
    )*};
}

float_undo_eq!(f32, f64);

macro_rules! exact_undo_eq {
    ($($exact:ty),*) => {$(
        impl UndoEq for $exact {
            fn undo_eq(&self, other: &Self) -> bool {
                self == other
            }
        }
    )*};
}

exact_undo_eq!(
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    String,
    &str,
    ()
);

impl<T: UndoEq> UndoEq for Option<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Some(a), Some(b)) => a.undo_eq(b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: UndoEq> UndoEq for [T] {
    fn undo_eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.undo_eq(b))
    }
}

impl<T: UndoEq> UndoEq for Vec<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        self[..].undo_eq(&other[..])
    }
}

impl<T: UndoEq, const N: usize> UndoEq for [T; N] {
    fn undo_eq(&self, other: &Self) -> bool {
        self[..].undo_eq(&other[..])
    }
}

impl<T: UndoEq + ?Sized> UndoEq for Box<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        (**self).undo_eq(other)
    }
}

impl<T: UndoEq + ?Sized> UndoEq for Rc<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        (**self).undo_eq(other)
    }
}

impl<T: UndoEq + ?Sized> UndoEq for Arc<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        (**self).undo_eq(other)
    }
}

impl<T: UndoEq> UndoEq for VecDeque<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.undo_eq(b))
    }
}

impl<K: Ord, V: UndoEq> UndoEq for BTreeMap<K, V> {
    fn undo_eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other)
                .all(|((ka, a), (kb, b))| ka == kb && a.undo_eq(b))
    }
}

impl<K: Eq + Hash, V: UndoEq, S: BuildHasher> UndoEq for HashMap<K, V, S> {
    fn undo_eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, a)| other.get(key).is_some_and(|b| a.undo_eq(b)))
    }
}

// A set's elements are `Eq`, whose `==` is reflexive, so `==` is enough.
impl<T: Ord> UndoEq for BTreeSet<T> {
    fn undo_eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: Eq + Hash, S: BuildHasher> UndoEq for HashSet<T, S> {
    fn undo_eq(&self, other: &Self) -> bool {
        self == other
    }
}

macro_rules! tuple_undo_eq {
    ($(($($name:ident $index:tt),+)),*) => {$(
        impl<$($name: UndoEq),+> UndoEq for ($($name,)+) {
            fn undo_eq(&self, other: &Self) -> bool {
                $(self.$index.undo_eq(&other.$index))&&+
            }
        }
    )*};
}

tuple_undo_eq!(
    (A 0),
    (A 0, B 1),
    (A 0, B 1, C 2),
    (A 0, B 1, C 2, D 3),
    (A 0, B 1, C 2, D 3, E 4),
    (A 0, B 1, C 2, D 3, E 4, F 5),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
);

/// Implements [`UndoEq`] by `==`, and [`UndoSize`](crate::UndoSize) as the
/// type's own size, for a type whose `==` is reflexive and that owns nothing
/// on the heap: an enum of plain data, an `Eq` newtype.
///
/// The macro checks both at compile time by requiring [`Eq`] and [`Copy`],
/// as the examples show. For a struct that fails either, use
/// [`undo_by_fields!`](crate::undo_by_fields); for a generic type, implement
/// the traits by hand.
///
/// # Examples
///
/// ```
/// use seam_undo::{UndoEq, undo_by_eq};
///
/// #[derive(Clone, Copy, PartialEq, Eq)]
/// enum Mode {
///     Linear,
///     Log,
/// }
/// undo_by_eq!(Mode);
///
/// assert!(Mode::Log.undo_eq(&Mode::Log));
/// assert!(!Mode::Log.undo_eq(&Mode::Linear));
/// ```
///
/// A type that is only `PartialEq` does not compile, since `==` is not
/// reflexive for a NaN:
///
/// ```compile_fail
/// use seam_undo::undo_by_eq;
///
/// #[derive(Clone, Copy, PartialEq)]
/// struct Gain(f32);
/// undo_by_eq!(Gain);
/// ```
///
/// A type that is not `Copy` does not compile, since it may own heap memory
/// the history's bound would not count:
///
/// ```compile_fail
/// use seam_undo::undo_by_eq;
///
/// #[derive(PartialEq, Eq)]
/// struct Label(String);
/// undo_by_eq!(Label);
/// ```
#[macro_export]
macro_rules! undo_by_eq {
    ($($ty:ty),+ $(,)?) => {$(
        impl $crate::UndoEq for $ty {
            fn undo_eq(&self, other: &Self) -> bool {
                self == other
            }
        }

        impl $crate::UndoSize for $ty {}

        const _: fn() = || {
            // `Copy` owns nothing on the heap, which the default size relies
            // on.
            fn assert_plain<T: ::core::cmp::Eq + ::core::marker::Copy>() {}
            assert_plain::<$ty>();
        };
    )+};
}

/// Implements [`UndoEq`] and [`UndoSize`](crate::UndoSize) for a struct from
/// its fields. Two values are equal when every field is, and a value books its
/// own size plus what the fields own on the heap.
///
/// The list must name every field: a field left out fails to compile, where
/// ignoring it would drop the undo step for a change to it. For a tuple
/// struct or a generic one, implement the traits by hand.
///
/// # Examples
///
/// ```
/// use seam_undo::{UndoEq, undo_by_fields};
///
/// struct Band {
///     freq: f64,
///     label: String,
/// }
/// undo_by_fields!(Band { freq, label });
///
/// let a = Band { freq: f64::NAN, label: "low".into() };
/// let b = Band { freq: f64::NAN, label: "low".into() };
/// assert!(a.undo_eq(&b));
/// ```
///
/// A field left out does not compile:
///
/// ```compile_fail
/// use seam_undo::undo_by_fields;
///
/// struct Band {
///     freq: f64,
///     label: String,
/// }
/// undo_by_fields!(Band { freq });
/// ```
#[macro_export]
macro_rules! undo_by_fields {
    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl $crate::UndoEq for $ty {
            fn undo_eq(&self, other: &Self) -> bool {
                // Naming every field makes a missing one a compile error.
                let Self { $($field: _),* } = self;
                true $(&& $crate::UndoEq::undo_eq(&self.$field, &other.$field))*
            }
        }

        impl $crate::UndoSize for $ty {
            fn undo_size(&self) -> usize {
                ::core::mem::size_of::<Self>() $(+ $crate::UndoSize::heap_size(&self.$field))*
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::UndoEq;

    #[test]
    fn a_nan_equals_itself_and_zero_equals_minus_zero() {
        let payload = f32::from_bits(0x7FC0_1234);
        assert!(payload.undo_eq(&payload));
        assert!(f64::NAN.undo_eq(&f64::NAN));
        assert!(!f32::NAN.undo_eq(&payload), "two different NaNs differ");
        assert!(0.0_f32.undo_eq(&-0.0));
        assert!(0.0_f64.undo_eq(&-0.0));
        assert!(!1.0_f32.undo_eq(&1.000_001));
        assert!(vec![1.0, f64::NAN].undo_eq(&vec![1.0, f64::NAN]));
        assert!(!vec![1.0].undo_eq(&vec![1.0, 2.0]));
        assert!(Some((f32::NAN, 3_u8)).undo_eq(&Some((f32::NAN, 3))));
        assert!(!Some(1_u8).undo_eq(&None));
    }

    #[test]
    fn the_standard_collections_compare_their_elements_with_undo_eq() {
        use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
        use std::rc::Rc;
        use std::sync::Arc;

        assert!(Rc::new(f64::NAN).undo_eq(&Rc::new(f64::NAN)));
        assert!(Arc::new([f32::NAN]).undo_eq(&Arc::new([f32::NAN])));
        assert!(VecDeque::from([f64::NAN, 1.0]).undo_eq(&VecDeque::from([f64::NAN, 1.0])));
        let nan_map = |key: &'static str| BTreeMap::from([(key, f64::NAN)]);
        assert!(nan_map("a").undo_eq(&nan_map("a")));
        assert!(!nan_map("a").undo_eq(&nan_map("b")));
        let hash_map = |value: f64| HashMap::from([("a", value), ("b", 1.0)]);
        assert!(hash_map(f64::NAN).undo_eq(&hash_map(f64::NAN)));
        assert!(!hash_map(2.0).undo_eq(&hash_map(3.0)));
        assert!(HashSet::from([1_u8, 2]).undo_eq(&HashSet::from([2, 1])));
        assert!((1_u8, 2_u8, 3_u8, 4_u8, 5_u8, f64::NAN).undo_eq(&(1, 2, 3, 4, 5, f64::NAN)));
    }

    #[test]
    fn the_macros_write_the_impls() {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mode {
            A,
            B,
        }
        crate::undo_by_eq!(Mode);
        assert!(Mode::A.undo_eq(&Mode::A));
        assert!(!Mode::A.undo_eq(&Mode::B));

        struct Band {
            freq: f64,
            label: String,
            taps: Vec<u8>,
        }
        crate::undo_by_fields!(Band { freq, label, taps });
        let band = |freq: f64, taps: usize| Band {
            freq,
            label: "x".into(),
            taps: vec![0; taps],
        };
        assert!(band(f64::NAN, 2).undo_eq(&band(f64::NAN, 2)));
        assert!(!band(1.0, 2).undo_eq(&band(1.0, 3)));
        let size = crate::UndoSize::undo_size(&band(1.0, 100));
        assert!(size >= size_of::<Band>() + 100, "the vector's heap counts");
    }
}
