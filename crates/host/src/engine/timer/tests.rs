use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use super::*;

#[test]
fn the_timer_ticks_through_the_threads_loop_until_dropped() {
    let ticks = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&ticks);
    let timer = Timer::start(Box::new(move || counter.set(counter.get() + 1))).expect("a timer");
    pump(Duration::from_millis(400));
    // About 25 ticks at about 60 a second; a loaded CI machine may manage
    // fewer.
    assert!(ticks.get() >= 5, "{} ticks in 400 ms", ticks.get());
    drop(timer);
    let stopped = ticks.get();
    pump(Duration::from_millis(100));
    assert_eq!(ticks.get(), stopped, "no tick after the drop");
}

#[test]
fn an_abandoned_timer_ticks_no_more() {
    let ticks = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&ticks);
    let timer = Timer::start(Box::new(move || counter.set(counter.get() + 1))).expect("a timer");
    let handle = timer.handle();
    std::thread::spawn(move || abandon(handle))
        .join()
        .expect("abandoned from another thread");
    pump(Duration::from_millis(100));
    assert_eq!(ticks.get(), 0);
    drop(timer);
}

#[cfg(windows)]
#[test]
fn an_abandoned_timer_s_window_stops_it_at_its_next_message() {
    let timer = Timer::start(Box::new(|| {})).expect("a timer");
    let handle = timer.handle();
    assert!(!imp::stopped(handle), "a running timer's message is due");
    std::thread::spawn(move || abandon(handle))
        .join()
        .expect("abandoned from another thread");
    pump(Duration::from_millis(100));
    assert!(
        imp::stopped(handle),
        "none after the one that found no tick"
    );
    drop(timer);
}
