//! The gate itself: it counts what it should and nothing else.

seam_testkit::install_alloc_gate!();

use seam_testkit::alloc::{allocations_in, assert_no_allocations, permit_alloc};

#[test]
fn counts_an_allocation_and_its_free() {
    let ((), count) = allocations_in(|| drop(std::hint::black_box(vec![1u8; 16])));
    assert_eq!(count, 2);
}

#[test]
fn passes_code_that_does_not_allocate() {
    let sum = assert_no_allocations("summing a fixed array", || {
        std::hint::black_box([1.0f32; 64]).iter().sum::<f32>()
    });
    assert_eq!(sum, 64.0);
}

#[test]
fn an_explicit_permit_is_not_counted() {
    let ((), count) = allocations_in(|| permit_alloc(|| drop(std::hint::black_box(Box::new(1)))));
    assert_eq!(count, 0);
}
