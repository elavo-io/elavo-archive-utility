//! Extraction folders: dot-rooted tarballs, cleanup after a failure, and
//! .deb packages unpacked past their data tarball.

use std::fs;
use std::path::Path;
use std::process::Command;

use elavo_archive_utility_core::{
    compress, extract, Cancel, CompressRequest, Conflict, ExtractRequest, Format, NewFolder,
};

fn has(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

fn request(archive: &Path, destination: &Path, new_folder: NewFolder) -> ExtractRequest {
    ExtractRequest {
        archive: archive.to_path_buf(),
        destination: destination.to_path_buf(),
        password: None,
        new_folder,
        conflict: Conflict::Rename,
    }
}

#[test]
fn a_dot_rooted_tarball_extracts_into_its_own_folder() {
    if !has("tar") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("plain");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("a.txt"), b"a\n").unwrap();
    fs::write(source.join("b.txt"), b"b\n").unwrap();
    let archive = dir.path().join("plain.tar");
    assert!(Command::new("tar")
        .args([
            "-C",
            source.to_str().unwrap(),
            "-cf",
            archive.to_str().unwrap(),
            "."
        ])
        .status()
        .unwrap()
        .success());
    let dest = dir.path().join("out");
    fs::create_dir_all(&dest).unwrap();
    let outcome = extract(
        &request(&archive, &dest, NewFolder::WhenSeveral),
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(outcome.path, dest.join("plain"));
    assert_eq!(fs::read(dest.join("plain/a.txt")).unwrap(), b"a\n");
    assert_eq!(fs::read(dest.join("plain/b.txt")).unwrap(), b"b\n");
    assert!(!dest.join("a.txt").exists());
}

#[test]
fn a_failed_extraction_leaves_no_folder_behind() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("fake.zip");
    fs::write(&archive, b"PK\x03\x04 this is not really a zip archive").unwrap();
    let dest = dir.path().join("out");
    fs::create_dir_all(&dest).unwrap();
    let result = extract(
        &request(&archive, &dest, NewFolder::Always),
        &Cancel::new(),
        |_| {},
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read_dir(&dest).unwrap().count(),
        0,
        "{dest:?} is not empty"
    );
}

#[test]
fn a_cancelled_extraction_leaves_no_folder_behind() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    fs::write(&file, b"hello\n").unwrap();
    let archive = dir.path().join("hello.zip");
    compress(
        &CompressRequest {
            inputs: vec![file],
            output: archive.clone(),
            format: Format::Zip,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    let dest = dir.path().join("out");
    fs::create_dir_all(&dest).unwrap();
    let cancel = Cancel::new();
    let mut first = true;
    let result = extract(
        &request(&archive, &dest, NewFolder::Always),
        &cancel,
        |_| {
            if first {
                first = false;
                cancel.cancel();
            }
        },
    );
    if result.is_err() {
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }
}

#[test]
fn an_existing_folder_is_kept_when_extraction_fails() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("fake.zip");
    fs::write(&archive, b"PK\x03\x04 not a zip").unwrap();
    let dest = dir.path().join("out");
    fs::create_dir_all(dest.join("fake")).unwrap();
    fs::write(dest.join("fake/keep.txt"), b"keep\n").unwrap();
    assert!(extract(
        &request(&archive, &dest, NewFolder::Always),
        &Cancel::new(),
        |_| {}
    )
    .is_err());
    assert_eq!(fs::read(dest.join("fake/keep.txt")).unwrap(), b"keep\n");
}

fn ar_member(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        name,
        0,
        0,
        0,
        "100644",
        data.len()
    );
    assert_eq!(header.len(), 60);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(b'\n');
    }
}

#[test]
fn a_deb_unpacks_its_files_and_control_files() {
    if !has("tar") || !has("xz") || !has("7z") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    fs::create_dir_all(data.join("usr/share/hi")).unwrap();
    fs::write(data.join("usr/share/hi/hello.txt"), b"hello\n").unwrap();
    let control = dir.path().join("control");
    fs::create_dir_all(&control).unwrap();
    fs::write(control.join("control"), b"Package: hi\n").unwrap();
    let data_tar = dir.path().join("data.tar.xz");
    let control_tar = dir.path().join("control.tar.xz");
    for (from, to) in [(&data, &data_tar), (&control, &control_tar)] {
        assert!(Command::new("tar")
            .args([
                "-C",
                from.to_str().unwrap(),
                "-cJf",
                to.to_str().unwrap(),
                "."
            ])
            .status()
            .unwrap()
            .success());
    }
    let mut deb = b"!<arch>\n".to_vec();
    ar_member(&mut deb, "debian-binary", b"2.0\n");
    ar_member(&mut deb, "control.tar.xz", &fs::read(&control_tar).unwrap());
    ar_member(&mut deb, "data.tar.xz", &fs::read(&data_tar).unwrap());
    let archive = dir.path().join("hi.deb");
    fs::write(&archive, deb).unwrap();
    let dest = dir.path().join("out");
    fs::create_dir_all(&dest).unwrap();
    let outcome = extract(
        &request(&archive, &dest, NewFolder::Always),
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(outcome.path, dest.join("hi"));
    assert_eq!(
        fs::read(dest.join("hi/usr/share/hi/hello.txt")).unwrap(),
        b"hello\n"
    );
    assert_eq!(
        fs::read(dest.join("hi/DEBIAN/control")).unwrap(),
        b"Package: hi\n"
    );
    assert!(!dest.join("hi/data.tar").exists());
}

#[test]
fn separate_archives_go_into_a_named_folder() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("one.txt");
    let b = dir.path().join("two.txt");
    fs::write(&a, b"1\n").unwrap();
    fs::write(&b, b"2\n").unwrap();
    let folder = dir.path().join("opt");
    fs::create_dir_all(&folder).unwrap();
    compress(
        &CompressRequest {
            inputs: vec![a, b],
            output: folder.clone(),
            format: Format::Zip,
            separate: true,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert!(folder.join("one.zip").is_file());
    assert!(folder.join("two.zip").is_file());
    assert!(!dir.path().join("one.zip").exists());
}
