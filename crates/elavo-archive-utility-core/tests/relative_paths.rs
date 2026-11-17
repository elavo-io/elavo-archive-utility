//! Relative paths, as typed on a command line. This runs in its own test
//! binary because it changes the working directory.

use std::fs;

use elavo_archive_utility_core::{compress, Cancel, CompressRequest, Format};

#[test]
fn a_relative_output_is_written_and_verified() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), b"hello\n").unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    for (format, name) in [
        (Format::SevenZ, "v.7z"),
        (Format::Zip, "v.zip"),
        (Format::TarGz, "v.tar.gz"),
    ] {
        let outcome = compress(
            &CompressRequest {
                inputs: vec!["hello.txt".into()],
                output: name.into(),
                format,
                verify: true,
                ..CompressRequest::default()
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(outcome.path.is_absolute(), "{:?}", outcome.path);
        assert!(dir.path().join(name).is_file(), "{name}");
    }
}
