//! Contract assertions as test failures.
//!
//! Library code checks a caller's contract with `debug_assert!`, which panics
//! in every test build but the release run of the allocation gates
//! (`--release --test alloc_gate`). A panic on the test thread, or on a thread
//! the test scopes or joins, fails the test, so a broken contract cannot pass
//! as a line in a log.
//!
//! A test that breaks a contract on purpose wraps the call in
//! [`expect_contract_panic`], naming the assertion it expects, so an assertion
//! that vanishes from the library fails its test too. Such a test is
//! `#[cfg(debug_assertions)]`: a release build compiles the assertion out.

use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f`, which must hit a contract assertion whose message contains
/// `expected`, and returns the whole message.
///
/// # Panics
///
/// If `f` returned without panicking, or panicked with a message that does
/// not contain `expected`.
///
/// # Examples
///
/// ```
/// use seam_testkit::contract::expect_contract_panic;
///
/// // A contract assertion in the library under test.
/// fn set_gain(db: f32) {
///     assert!(db.is_finite(), "gain must be finite");
/// }
///
/// let message = expect_contract_panic("must be finite", || set_gain(f32::NAN));
/// assert!(message.contains("gain"));
/// ```
pub fn expect_contract_panic(expected: &str, f: impl FnOnce()) -> String {
    let Err(payload) = catch_unwind(AssertUnwindSafe(f)) else {
        panic!("expected a contract assertion containing {expected:?}, and none fired");
    };
    let message = payload
        .downcast_ref::<&str>()
        .map(|&message| message.to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(
        message.contains(expected),
        "expected a contract assertion containing {expected:?}, and got {message:?}"
    );
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_message_of_the_assertion_that_fired() {
        let message = expect_contract_panic("arithmetic", || {
            assert!(std::hint::black_box(2) == 3, "arithmetic broke")
        });
        assert_eq!(message, "arithmetic broke");
    }

    #[test]
    fn returns_a_formatted_message_too() {
        let message = expect_contract_panic("broke at 3", || {
            let at = std::hint::black_box(3);
            assert!(at == 2, "arithmetic broke at {at}");
        });
        assert_eq!(message, "arithmetic broke at 3");
    }

    #[test]
    fn fails_when_no_assertion_fires() {
        let outer = catch_unwind(|| expect_contract_panic("arithmetic", || {}));
        assert!(outer.is_err());
    }

    #[test]
    fn fails_when_a_different_panic_fires() {
        let outer =
            catch_unwind(|| expect_contract_panic("arithmetic", || panic!("something else broke")));
        assert!(outer.is_err());
    }
}
