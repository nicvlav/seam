//! The audio-thread mark: each call that must stay off the audio thread
//! panics in a debug build while the mark is held, and stays silent off it.
//! In a release build there is no mark.

use std::sync::Arc;

use seam_params::{
    HostEventDrain, Kind, List, ParamDecl, ParamSet, ParamSetBuilder, Value, audio_thread,
};

fn set() -> Arc<ParamSet> {
    let mut builder = ParamSetBuilder::new();
    builder
        .add(ParamDecl::new("p", "p", Kind::Bool, 0.0))
        .expect("valid");
    builder.build()
}

#[test]
fn the_mark_nests_and_clears_with_the_last_guard() {
    assert!(!audio_thread::is_here());
    {
        let _outer = audio_thread::enter();
        let inner = audio_thread::enter();
        drop(inner);
        assert_eq!(audio_thread::is_here(), cfg!(debug_assertions));
    }
    assert!(!audio_thread::is_here());
}

#[test]
fn the_mark_belongs_to_its_thread() {
    let _audio = audio_thread::enter();
    std::thread::spawn(|| assert!(!audio_thread::is_here()))
        .join()
        .expect("the other thread");
}

#[test]
fn every_checked_call_is_silent_off_the_audio_thread() {
    let set = set();
    let id = set.id("p").expect("declared");
    let value = Value::new(1_i32, &set);
    value.set(2);
    assert_eq!(value.get(), 2);
    let list = List::new(vec![1_i32], &set);
    list.push(2);
    assert_eq!(list.len(), 2);
    drop(set.clock().reader().hold_loads());
    set.program().load().restore(id, 1.0);
    let ui = set.take_ui().expect("the first UI handle");
    drop(ui);
    let drain = set.take_host_event_drain().expect("the first drain");
    drop(drain);
    drop(set.audio());
}

#[cfg(debug_assertions)]
mod debug_checks {
    use super::*;

    #[test]
    #[should_panic(expected = "a Value call must not run on the audio thread")]
    fn a_value_call_is_refused_on_the_audio_thread() {
        let set = set();
        let value = Value::new(1_i32, &set);
        let _audio = audio_thread::enter();
        value.set(2);
    }

    #[test]
    #[should_panic(expected = "a Value call must not run on the audio thread")]
    fn a_value_read_is_refused_on_the_audio_thread() {
        let set = set();
        let value = Value::new(1_i32, &set);
        let _audio = audio_thread::enter();
        let _ = value.get();
    }

    #[test]
    #[should_panic(expected = "a List call must not run on the audio thread")]
    fn a_list_call_is_refused_on_the_audio_thread() {
        let set = set();
        let list = List::new(vec![1_i32], &set);
        let _audio = audio_thread::enter();
        list.push(2);
    }

    #[test]
    #[should_panic(expected = "holding off loads must not run on the audio thread")]
    fn holding_off_loads_is_refused_on_the_audio_thread() {
        let set = set();
        let reader = set.clock().reader();
        let _audio = audio_thread::enter();
        let _held = reader.hold_loads();
    }

    #[test]
    #[should_panic(expected = "Load::restore must not run on the audio thread")]
    fn a_load_is_refused_on_the_audio_thread() {
        let set = set();
        let id = set.id("p").expect("declared");
        let load = set.program().load();
        let _audio = audio_thread::enter();
        load.restore(id, 1.0);
    }

    #[test]
    #[should_panic(expected = "ParamSet::take_ui must not run on the audio thread")]
    fn taking_the_ui_handle_is_refused_on_the_audio_thread() {
        let set = set();
        let audio = audio_thread::enter();
        let ui = set.take_ui();
        drop(audio);
        drop(ui);
    }

    #[test]
    #[should_panic(expected = "ParamSet::take_host_event_drain must not run on the audio thread")]
    fn taking_the_host_event_drain_is_refused_on_the_audio_thread() {
        let set = set();
        let audio = audio_thread::enter();
        let drain = set.take_host_event_drain();
        drop(audio);
        drop(drain);
    }

    #[test]
    #[should_panic(expected = "dropping AudioParams must not run on the audio thread")]
    fn dropping_audio_params_is_refused_on_the_audio_thread() {
        let audio = set().audio();
        let _guard = audio_thread::enter();
        drop(audio);
    }

    #[test]
    #[should_panic(expected = "dropping a HostEventDrain must not run on the audio thread")]
    fn dropping_a_host_event_drain_is_refused_on_the_audio_thread() {
        let drain: HostEventDrain = set().take_host_event_drain().expect("the first drain");
        let _guard = audio_thread::enter();
        drop(drain);
    }

    #[test]
    fn a_panic_while_unwinding_is_not_made_worse() {
        // A drop during unwinding is not checked, so it cannot abort.
        let outcome = std::panic::catch_unwind(|| {
            let audio = set().audio();
            let _guard = audio_thread::enter();
            let _held = audio;
            panic!("an unrelated panic");
        });
        assert!(outcome.is_err());
    }
}

#[cfg(not(debug_assertions))]
mod release_has_no_mark {
    use super::*;

    #[test]
    fn the_guard_is_zero_sized_and_nothing_is_ever_refused() {
        assert_eq!(std::mem::size_of::<audio_thread::AudioThreadGuard>(), 0);
        let set = set();
        let value = Value::new(1_i32, &set);
        let audio = set.audio();
        let _guard = audio_thread::enter();
        assert!(!audio_thread::is_here());
        value.set(2);
        drop(audio);
    }
}
