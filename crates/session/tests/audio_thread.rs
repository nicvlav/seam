//! A session locks, so a debug build refuses its calls on the audio thread.

use seam_params::{ParamSetBuilder, audio_thread};
use seam_session::Session;

#[test]
fn a_save_is_silent_off_the_audio_thread() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    assert!(session.save().is_none());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a Session call must not run on the audio thread")]
fn a_save_is_refused_on_the_audio_thread() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let _audio = audio_thread::enter();
    let _ = session.save();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a Session call must not run on the audio thread")]
fn a_restore_is_refused_on_the_audio_thread() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let _audio = audio_thread::enter();
    session.restore(None);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_release_build_refuses_nothing() {
    let set = ParamSetBuilder::new().build();
    let session = Session::new(&set);
    let _audio = audio_thread::enter();
    let _ = session.save();
}
