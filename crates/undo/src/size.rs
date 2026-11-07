//! How much memory a stored value keeps alive, for the history's bound.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

/// What a value keeps alive, so the history's bound counts the heap and not
/// just the handle: a 100 KB `Vec<u8>` books 100 KB, not 24 bytes.
///
/// Every enrolled value and list element implements it. The default is
/// `size_of_val`, right for any type that owns nothing on the heap; a type
/// that does reports its heap too. An estimate is enough: the bound is soft.
///
/// # Examples
///
/// ```
/// use seam_undo::UndoSize;
///
/// // A type that owns heap memory reports it, so the bound counts a 1 MB
/// // sample as 1 MB.
/// struct Sample(Vec<f32>);
///
/// impl UndoSize for Sample {
///     fn undo_size(&self) -> usize {
///         size_of::<Self>() + self.0.capacity() * size_of::<f32>()
///     }
/// }
///
/// assert!(Sample(vec![0.0; 250_000]).undo_size() >= 1_000_000);
/// assert_eq!(7_u32.undo_size(), 4);
/// ```
pub trait UndoSize {
    /// Returns the bytes the value occupies: itself, and what it owns on the
    /// heap. The trait's example implements it.
    fn undo_size(&self) -> usize {
        size_of_val(self)
    }

    /// Returns the bytes the value owns beyond itself:
    /// [`undo_size`](Self::undo_size) less the value's own size, and zero if
    /// that would be negative.
    fn heap_size(&self) -> usize {
        self.undo_size().saturating_sub(size_of_val(self))
    }
}

macro_rules! flat_undo_size {
    ($($flat:ty),*) => {$(
        impl UndoSize for $flat {}
    )*};
}

flat_undo_size!(
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
    f32,
    f64,
    (),
    &str
);

impl UndoSize for String {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + self.capacity()
    }
}

impl UndoSize for str {}

/// The heap bytes `items` own. The container's own size already counts the
/// items themselves, so adding their full size would count them twice.
fn heap_of<'a, T: UndoSize + 'a>(items: impl IntoIterator<Item = &'a T>) -> usize {
    items.into_iter().map(UndoSize::heap_size).sum()
}

impl<T: UndoSize> UndoSize for Option<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + self.as_ref().map_or(0, UndoSize::heap_size)
    }
}

impl<T: UndoSize> UndoSize for [T] {
    fn undo_size(&self) -> usize {
        size_of_val(self) + heap_of(self)
    }
}

impl<T: UndoSize, const N: usize> UndoSize for [T; N] {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + heap_of(self)
    }
}

impl<T: UndoSize> UndoSize for Vec<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + self.capacity() * size_of::<T>() + heap_of(self)
    }
}

impl<T: UndoSize> UndoSize for VecDeque<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + self.capacity() * size_of::<T>() + heap_of(self)
    }
}

impl<T: UndoSize + ?Sized> UndoSize for Box<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + (**self).undo_size()
    }
}

// A shared allocation counts in full for each handle a step holds: the
// bound errs towards keeping less.
impl<T: UndoSize + ?Sized> UndoSize for Rc<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + (**self).undo_size()
    }
}

impl<T: UndoSize + ?Sized> UndoSize for Arc<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + (**self).undo_size()
    }
}

impl<K: UndoSize, V: UndoSize> UndoSize for BTreeMap<K, V> {
    fn undo_size(&self) -> usize {
        size_of::<Self>()
            + self.len() * size_of::<(K, V)>()
            + self
                .iter()
                .map(|(key, value)| key.heap_size() + value.heap_size())
                .sum::<usize>()
    }
}

impl<K: UndoSize, V: UndoSize, S> UndoSize for HashMap<K, V, S> {
    fn undo_size(&self) -> usize {
        size_of::<Self>()
            + self.capacity() * size_of::<(K, V)>()
            + self
                .iter()
                .map(|(key, value)| key.heap_size() + value.heap_size())
                .sum::<usize>()
    }
}

impl<T: UndoSize> UndoSize for BTreeSet<T> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + self.len() * size_of::<T>() + heap_of(self)
    }
}

impl<T: UndoSize, S> UndoSize for HashSet<T, S> {
    fn undo_size(&self) -> usize {
        size_of::<Self>() + self.capacity() * size_of::<T>() + heap_of(self)
    }
}

macro_rules! tuple_undo_size {
    ($(($($name:ident $index:tt),+)),*) => {$(
        impl<$($name: UndoSize),+> UndoSize for ($($name,)+) {
            fn undo_size(&self) -> usize {
                size_of::<Self>() $(+ self.$index.heap_size())+
            }
        }
    )*};
}

tuple_undo_size!(
    (A 0),
    (A 0, B 1),
    (A 0, B 1, C 2),
    (A 0, B 1, C 2, D 3),
    (A 0, B 1, C 2, D 3, E 4),
    (A 0, B 1, C 2, D 3, E 4, F 5),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
);

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
    use std::rc::Rc;
    use std::sync::Arc;

    use super::UndoSize;

    #[test]
    fn a_pointer_books_itself_and_all_it_points_to() {
        let boxed = Box::new(vec![0_u8; 100]);
        assert_eq!(
            boxed.undo_size(),
            size_of::<Box<Vec<u8>>>() + size_of::<Vec<u8>>() + 100
        );
        let slice: Box<[u8]> = vec![0; 10].into_boxed_slice();
        assert_eq!(slice.undo_size(), size_of::<Box<[u8]>>() + 10);
        let rc = Rc::new(String::from("abcd"));
        assert_eq!(
            rc.undo_size(),
            size_of::<Rc<String>>() + size_of::<String>() + rc.capacity()
        );
        let arc = Arc::new([0_u16; 8]);
        assert_eq!(arc.undo_size(), size_of::<Arc<[u16; 8]>>() + 16);
    }

    #[test]
    fn a_deque_books_its_capacity_and_its_elements_heap() {
        let mut deque = VecDeque::with_capacity(4);
        deque.push_back(String::from("abc"));
        assert_eq!(
            deque.undo_size(),
            size_of::<VecDeque<String>>() + deque.capacity() * size_of::<String>() + 3
        );
    }

    #[test]
    fn a_map_books_its_entries_and_their_heap() {
        let entries =
            [("ab", 10), ("c", 20)].map(|(key, len)| (String::from(key), vec![0_u8; len]));
        let heap: usize = entries
            .iter()
            .map(|(key, value)| key.capacity() + value.capacity())
            .sum();
        let entry = size_of::<(String, Vec<u8>)>();
        let tree: BTreeMap<_, _> = entries.clone().into();
        assert_eq!(
            tree.undo_size(),
            size_of::<BTreeMap<String, Vec<u8>>>() + 2 * entry + heap
        );
        let hash: HashMap<_, _> = entries.into();
        assert_eq!(
            hash.undo_size(),
            size_of::<HashMap<String, Vec<u8>>>() + hash.capacity() * entry + heap
        );
    }

    #[test]
    fn a_set_books_its_elements_and_their_heap() {
        let elements = ["ab", "cde"].map(String::from);
        let tree: BTreeSet<_> = elements.clone().into();
        assert_eq!(
            tree.undo_size(),
            size_of::<BTreeSet<String>>() + 2 * size_of::<String>() + 5
        );
        let hash: HashSet<_> = elements.into();
        assert_eq!(
            hash.undo_size(),
            size_of::<HashSet<String>>() + hash.capacity() * size_of::<String>() + 5
        );
    }

    #[test]
    fn an_array_and_a_slice_book_their_elements_heap() {
        let array = [vec![0_u8; 3], vec![0_u8; 5]];
        assert_eq!(array.undo_size(), size_of::<[Vec<u8>; 2]>() + 8);
        let slice: &[Vec<u8>] = &array;
        assert_eq!(slice.undo_size(), size_of::<[Vec<u8>; 2]>() + 8);
    }

    #[test]
    fn a_container_books_its_heap() {
        assert_eq!(1_u8.undo_size(), 1);
        let bytes = vec![0_u8; 64 * 1024];
        assert_eq!(bytes.undo_size(), size_of::<Vec<u8>>() + 64 * 1024);
        let names = vec![String::from("abcd"); 10];
        assert!(names.undo_size() >= size_of::<Vec<String>>() + 10 * (size_of::<String>() + 4));
        assert_eq!(
            (1_u8, vec![0_u8; 100]).undo_size(),
            size_of::<(u8, Vec<u8>)>() + 100
        );
        assert_eq!(
            Some(vec![0_u8; 8]).undo_size(),
            size_of::<Option<Vec<u8>>>() + 8
        );
        assert_eq!(None::<Vec<u8>>.undo_size(), size_of::<Option<Vec<u8>>>());
    }
}
