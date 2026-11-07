//! The drift gate for every generated file: each committed file matches what
//! `cargo xtask gen` writes now.
#![cfg(feature = "generate")]

#[test]
fn every_generated_file_is_current() {
    let root = xtask::root();
    let files = xtask::generated();
    assert!(
        !files.is_empty(),
        "xtask generates no file, so this gate would check nothing"
    );
    for file in files {
        seam_testkit::drift::check_generated(&root.join(file.path), &file.contents);
    }
}
