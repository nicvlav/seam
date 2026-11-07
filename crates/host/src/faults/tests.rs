//! The faults' test plumbing: what each thread counted and wrote, so tests
//! running in parallel see only their own, and the faults' own tests.

use std::cell::{Cell, RefCell};

use super::*;

thread_local! {
    /// The faults counted on this thread.
    static THREAD: Cell<u64> = const { Cell::new(0) };
    /// The lines written on this thread, which a test reads in place of a
    /// debugger or stderr.
    static WRITTEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Counts a fault on this thread.
pub(super) fn counted() {
    THREAD.with(|count| count.set(count.get() + 1));
}

/// Keeps a line written on this thread.
pub(super) fn wrote(line: &str) {
    WRITTEN.with(|written| written.borrow_mut().push(line.to_owned()));
}

/// The faults counted on this thread so far.
pub(crate) fn thread_faults() -> u64 {
    THREAD.with(Cell::get)
}

/// The lines written on this thread since the last call.
pub(crate) fn written() -> Vec<String> {
    WRITTEN.with(|written| std::mem::take(&mut *written.borrow_mut()))
}

#[test]
fn an_instance_reports_each_kind_once_and_counts_every_fault() {
    let faults = Faults::default();
    let total = boundary_faults();
    written();
    faults.record(Fault::AudioPanic);
    faults.record(Fault::AudioPanic);
    assert_eq!(written(), Vec::<String>::new(), "nothing written to record");
    faults.report("Probe (instance 1)");
    let lines = written();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("seam: Probe (instance 1): the processor panicked"));
    faults.record(Fault::AudioPanic);
    faults.record(Fault::Panic);
    faults.report("Probe (instance 1)");
    let lines = written();
    assert_eq!(lines.len(), 1, "only the new kind: {lines:?}");
    assert!(lines[0].contains("a call from the host panicked"));
    assert_eq!(faults.count(), 4);
    assert!(
        boundary_faults() >= total + 4,
        "counted for the process too"
    );
}

#[test]
fn every_fault_s_text_is_one_line_spaced_once() {
    for fault in Fault::ALL {
        let text = fault.what();
        assert!(
            !text.contains('\n') && !text.contains("  "),
            "{fault:?}: {text:?}"
        );
    }
}

#[test]
fn a_fault_recorded_on_another_thread_is_reported_on_this_one() {
    let faults = Faults::default();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            faults.record(Fault::WrongThread);
            assert_eq!(written(), Vec::<String>::new());
        });
    });
    assert_eq!(faults.count(), 1);
    faults.report("Probe (instance 2)");
    assert_eq!(written().len(), 1);
}
