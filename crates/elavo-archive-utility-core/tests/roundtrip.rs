use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use elavo_archive_utility_core::{
    compress, detect, extract, Cancel, CompressRequest, Conflict, Error, ExtractRequest, Format,
    Level, NewFolder,
};

fn tree(root: &Path) {
    fs::create_dir_all(root.join("sub/nested")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::write(root.join("hello.txt"), b"hello elavo-archive-utility\n").unwrap();
    fs::write(root.join("sub/caf\u{e9}.txt"), "caf\u{e9}\n".as_bytes()).unwrap();
    fs::write(root.join("sub/nested/bin.dat"), [0u8, 1, 2, 255, 10]).unwrap();
    fs::write(root.join("space name.txt"), b"spaces\n").unwrap();
}

fn assert_same(left: &Path, right: &Path) {
    fn walk(left: &Path, right: &Path) {
        let left_meta = fs::symlink_metadata(left).unwrap_or_else(|err| panic!("{left:?}: {err}"));
        let right_meta =
            fs::symlink_metadata(right).unwrap_or_else(|err| panic!("{right:?}: {err}"));
        assert_eq!(
            left_meta.file_type().is_symlink(),
            right_meta.file_type().is_symlink(),
            "{left:?}"
        );
        if left_meta.file_type().is_symlink() {
            assert_eq!(fs::read_link(left).unwrap(), fs::read_link(right).unwrap());
            return;
        }
        if left_meta.is_dir() {
            let mut a: Vec<_> = fs::read_dir(left)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            let mut b: Vec<_> = fs::read_dir(right)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "entries in {left:?}");
            for name in a {
                walk(&left.join(&name), &right.join(&name));
            }
            return;
        }
        assert_eq!(
            fs::read(left).unwrap(),
            fs::read(right).unwrap(),
            "{left:?}"
        );
    }
    walk(left, right);
}

fn round_trip(format: Format, level: Level) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Notes");
    tree(&src);
    let archive = dir.path().join(format!("out.{}", format.extension()));
    compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: archive.clone(),
            format,
            level,
            password: None,
            encrypt_names: false,
            split: None,
        ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("{format:?}: {err}"));
    assert!(archive.is_file(), "{archive:?} was not created");
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("extract {format:?}: {err}"));
    assert_same(&src, &dest.join("Notes"));
}

#[test]
fn tar_family_round_trips() {
    for format in [
        Format::Tar,
        Format::TarGz,
        Format::TarXz,
        Format::TarBz2,
        Format::TarZst,
    ] {
        round_trip(format, Level::Fast);
    }
}

#[test]
fn single_streams_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("hello.txt");
    fs::write(&src, b"hello elavo-archive-utility\n").unwrap();
    for format in [Format::Gz, Format::Xz, Format::Bz2, Format::Zst] {
        let archive = dir.path().join(format!("hello.txt.{}", format.extension()));
        compress(
            &CompressRequest {
                inputs: vec![src.clone()],
                output: archive.clone(),
                format,
                level: Level::Fast,
                password: None,
                encrypt_names: false,
                split: None,
                ..CompressRequest::default()
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap();
        let dest = dir.path().join(format!("out-{}", format.extension()));
        fs::create_dir(&dest).unwrap();
        extract(
            &ExtractRequest {
                archive,
                destination: dest.clone(),
                password: None,
                new_folder: NewFolder::Never,
                conflict: Conflict::Replace,
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(dest.join("hello.txt")).unwrap(),
            b"hello elavo-archive-utility\n"
        );
    }
}

#[test]
fn tar_keeps_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Notes");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("hello.txt"), b"hello\n").unwrap();
    std::os::unix::fs::symlink("hello.txt", src.join("link")).unwrap();
    let archive = dir.path().join("notes.tar");
    compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: archive.clone(),
            format: Format::Tar,
            level: Level::Store,
            password: None,
            encrypt_names: false,
            split: None,
        ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(
        fs::read_link(dest.join("Notes/link")).unwrap(),
        PathBuf::from("hello.txt")
    );
    assert_eq!(fs::read(dest.join("Notes/hello.txt")).unwrap(), b"hello\n");
}

#[test]
fn a_hostile_tar_cannot_escape_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let archive_path = dir.path().join("bad.tar");
    // The tar crate refuses to write `..` itself, so the fixture is a
    // hand-built ustar header. Extraction must not create the file outside.
    fs::write(&archive_path, ustar_with_name(b"../escaped.txt", b"pwn!")).unwrap();
    let dest = dir.path().join("out");
    fs::create_dir(&dest).unwrap();
    let err = extract(
        &ExtractRequest {
            archive: archive_path,
            destination: dest,
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    );
    assert!(err.is_err(), "a path with .. was accepted");
    assert!(!dir.path().join("escaped.txt").exists());
}

#[test]
fn cancel_before_starting_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("hello.txt");
    fs::write(&src, b"hello\n").unwrap();
    let cancel = Cancel::new();
    cancel.cancel();
    let err = compress(
        &CompressRequest {
            inputs: vec![src],
            output: dir.path().join("hello.tar.gz"),
            format: Format::TarGz,
            level: Level::Slowest,
            password: None,
            encrypt_names: false,
            split: None,
        ..CompressRequest::default()
        },
        &cancel,
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err}");
}

#[test]
fn seven_zip_and_zip_round_trip() {
    for format in [Format::SevenZ, Format::Zip] {
        round_trip(format, Level::Store);
    }
}

#[test]
fn passwords_round_trip_and_a_wrong_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Notes");
    tree(&src);
    for (format, encrypt_names) in [(Format::SevenZ, true), (Format::Zip, false)] {
        let archive = dir.path().join(format!("secret.{}", format.extension()));
        compress(
            &CompressRequest {
                inputs: vec![src.clone()],
                output: archive.clone(),
                format,
                level: Level::Store,
                password: Some("correct horse".into()),
                encrypt_names,
                split: None,
                ..CompressRequest::default()
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap_or_else(|err| panic!("compress {format:?}: {err}"));

        let missing = extract(
            &ExtractRequest {
                archive: archive.clone(),
                destination: dir.path().join("nope"),
                password: None,
                new_folder: NewFolder::Never,
                conflict: Conflict::Replace,
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            matches!(missing, Error::PasswordRequired | Error::WrongPassword),
            "{format:?} without a password: {missing}"
        );

        let wrong = extract(
            &ExtractRequest {
                archive: archive.clone(),
                destination: dir.path().join("wrong"),
                password: Some("nope".into()),
                new_folder: NewFolder::Never,
                conflict: Conflict::Replace,
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(
            matches!(wrong, Error::WrongPassword | Error::PasswordRequired),
            "{format:?} wrong password: {wrong}"
        );

        let dest = dir.path().join(format!("ok-{}", format.extension()));
        fs::create_dir(&dest).unwrap();
        extract(
            &ExtractRequest {
                archive,
                destination: dest.clone(),
                password: Some("correct horse".into()),
                new_folder: NewFolder::Never,
                conflict: Conflict::Replace,
            },
            &Cancel::new(),
            |_| {},
        )
        .unwrap_or_else(|err| panic!("extract {format:?}: {err}"));
        assert_same(&src, &dest.join("Notes"));
    }
}

#[test]
fn split_volumes_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("blob.bin");
    fs::write(&src, vec![7u8; 200 * 1024]).unwrap();
    let archive = dir.path().join("blob.7z");
    let outcome = compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: archive.clone(),
            format: Format::SevenZ,
            level: Level::Store,
            password: None,
            encrypt_names: false,
            split: Some(64 * 1024),
        ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert!(
        outcome.path.extension().and_then(|e| e.to_str()) == Some("001"),
        "first volume was {:?}",
        outcome.path
    );
    assert!(dir.path().join("blob.7z.002").is_file());
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive: outcome.path,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(
        fs::read(dest.join("blob.bin")).unwrap(),
        vec![7u8; 200 * 1024]
    );
}

#[test]
fn rar_extracts_a_stored_archive() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("hello.rar");
    fs::write(&archive, stored_rar(b"hello.txt", b"hello\n")).unwrap();
    assert_eq!(detect(&archive), Some(Format::Rar));
    let dest = dir.path().join("out");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(dest.join("hello.txt")).unwrap(), b"hello\n");
}

#[test]
fn iso_extracts() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("disc");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("readme.txt"), b"from the disc\n").unwrap();
    let iso = dir.path().join("disc.iso");
    let status = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso.to_str().unwrap(),
            "-V",
            "ARCHIVE",
            src.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "xorriso failed");
    assert_eq!(detect(&iso), Some(Format::Iso));
    let dest = dir.path().join("out");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive: iso,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    let found = fs::read(dest.join("readme.txt"))
        .or_else(|_| fs::read(dest.join("disc/readme.txt")))
        .unwrap_or_else(|_| {
            // Some ISO writers nest the folder name, some do not.
            fn find(dir: &Path) -> Option<Vec<u8>> {
                for entry in fs::read_dir(dir).ok()? {
                    let entry = entry.ok()?;
                    if entry.file_type().ok()?.is_dir() {
                        if let Some(bytes) = find(&entry.path()) {
                            return Some(bytes);
                        }
                    } else if entry.file_name() == "readme.txt" {
                        return fs::read(entry.path()).ok();
                    }
                }
                None
            }
            find(&dest).expect("readme.txt was not extracted")
        });
    assert_eq!(found, b"from the disc\n");
}

#[test]
fn rename_keeps_the_file_that_was_already_there() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("hello.txt");
    fs::write(&src, b"new\n").unwrap();
    let archive = dir.path().join("hello.tar");
    compress(
        &CompressRequest {
            inputs: vec![src],
            output: archive.clone(),
            format: Format::Tar,
            level: Level::Store,
            password: None,
            encrypt_names: false,
            split: None,
        ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    let dest = dir.path().join("out");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("hello.txt"), b"old\n").unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Rename,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(dest.join("hello.txt")).unwrap(), b"old\n");
    assert_eq!(fs::read(dest.join("hello (2).txt")).unwrap(), b"new\n");
}

fn has(tool: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join(tool).is_file())
    })
}

fn round_file(format: Format) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("hello.txt");
    fs::write(&src, b"hello elavo-archive-utility\n").unwrap();
    let archive = dir.path().join(format!("hello.txt.{}", format.extension()));
    compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: archive.clone(),
            format,
            level: Level::Fast,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("create {format:?}: {err}"));
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("extract {format:?}: {err}"));
    let found = fs::read(dest.join("hello.txt"))
        .or_else(|_| fs::read(dest.join("hello.txt.txt")))
        .unwrap_or_else(|_| panic!("missing payload for {format:?}"));
    assert_eq!(found, b"hello elavo-archive-utility\n");
}

#[test]
fn extra_compressors_round_trip() {
    round_file(Format::Snappy);
    if has("lz4") {
        round_file(Format::Lz4);
    }
    if has("brotli") {
        round_file(Format::Brotli);
    }
    if has("lzip") {
        round_file(Format::Lzip);
    }
    if has("lrzip") {
        round_file(Format::Lrzip);
    }
    if has("7z") || has("7zz") {
        round_file(Format::Wim);
    }
}

#[test]
fn a_folder_of_lz4_is_a_tar() {
    if !has("lz4") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Notes");
    tree(&src);
    let archive = dir.path().join("Notes.tar.lz4");
    compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: archive.clone(),
            format: Format::Lz4,
            level: Level::Fast,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(detect(&archive), Some(Format::TarLz4));
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(dest.join("Notes/hello.txt")).unwrap(), b"hello elavo-archive-utility\n");
}

#[test]
fn mac_metadata_is_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Notes");
    fs::create_dir_all(src.join("__MACOSX")).unwrap();
    fs::write(src.join("hello.txt"), b"hello\n").unwrap();
    fs::write(src.join(".DS_Store"), b"store").unwrap();
    fs::write(src.join("._hello.txt"), b"fork").unwrap();
    fs::write(src.join("__MACOSX/._hello.txt"), b"fork").unwrap();
    let archive = dir.path().join("Notes.tar");
    compress(
        &CompressRequest {
            inputs: vec![src],
            output: archive.clone(),
            format: Format::Tar,
            exclude_macos: true,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(dest.join("Notes/hello.txt")).unwrap(), b"hello\n");
    assert!(!dest.join("Notes/.DS_Store").exists());
    assert!(!dest.join("Notes/._hello.txt").exists());
    assert!(!dest.join("Notes/__MACOSX").exists());
}

#[test]
fn separate_archives_and_delete_after() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, b"a\n").unwrap();
    fs::write(&b, b"b\n").unwrap();
    compress(
        &CompressRequest {
            inputs: vec![a.clone(), b.clone()],
            output: dir.path().join("unused.tar"),
            format: Format::Tar,
            separate: true,
            delete_after: true,
            verify: true,
            exclude_macos: false,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert!(!a.exists() && !b.exists());
    assert!(dir.path().join("a.tar").is_file());
    assert!(dir.path().join("b.tar").is_file());
}

fn extract_ok(archive: &Path) -> PathBuf {
    let dest = archive.parent().unwrap().join(format!(
        "out-{}",
        archive.file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir_all(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive: archive.to_path_buf(),
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("extract {}: {err}", archive.display()));
    dest
}

fn find_named(dir: &Path, name: &str) -> Option<Vec<u8>> {
    for entry in fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if entry.file_name() == name {
            return fs::read(path).ok();
        }
        if path.is_dir() {
            if let Some(bytes) = find_named(&path, name) {
                return Some(bytes);
            }
        }
    }
    None
}

#[test]
fn open_formats_from_small_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let hello = dir.path().join("hello.txt");
    fs::write(&hello, b"hello elavo-archive-utility\n").unwrap();

    if has("ar") {
        let archive = dir.path().join("lib.a");
        assert!(Command::new("ar")
            .args(["rcs", archive.to_str().unwrap(), hello.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        assert_eq!(detect(&archive), Some(Format::Ar));
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
    }
    if has("cpio") {
        let archive = dir.path().join("files.cpio");
        let mut child = Command::new("cpio")
            .current_dir(dir.path())
            .args(["-o", "-H", "newc"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::from(fs::File::create(&archive).unwrap()))
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"hello.txt\n")
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(detect(&archive), Some(Format::Cpio));
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
    }
    if has("gcab") {
        let archive = dir.path().join("files.cab");
        assert!(Command::new("gcab")
            .current_dir(dir.path())
            .args(["-c", "-z", "files.cab", "hello.txt"])
            .status()
            .unwrap()
            .success());
        assert_eq!(detect(&archive), Some(Format::Cab));
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
    }
    if has("dpkg-deb") {
        let root = dir.path().join("pkg");
        fs::create_dir_all(root.join("DEBIAN")).unwrap();
        fs::write(
            root.join("DEBIAN/control"),
            "Package: hi\nVersion: 1.0\nArchitecture: all\nMaintainer: t <t@example.com>\nDescription: hi\n",
        )
        .unwrap();
        fs::write(root.join("hello.txt"), b"hello elavo-archive-utility\n").unwrap();
        let archive = dir.path().join("hi.deb");
        assert!(Command::new("dpkg-deb")
            .args(["--build", root.to_str().unwrap(), archive.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        assert_eq!(detect(&archive), Some(Format::Deb));
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
    }
    if has("compress") {
        let archive = dir.path().join("hello.Z");
        let mut child = Command::new("compress")
            .args(["-c", hello.to_str().unwrap()])
            .stdout(std::process::Stdio::from(fs::File::create(&archive).unwrap()))
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(detect(&archive), Some(Format::Compress));
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello").unwrap_or_else(|| find_named(&out, "hello.txt").unwrap()), b"hello elavo-archive-utility\n");
    }
    if has("lzma") || has("xz") {
        let archive = dir.path().join("hello.lzma");
        let status = if has("xz") {
            Command::new("xz")
                .args(["--format=lzma", "-c", hello.to_str().unwrap()])
                .stdout(std::process::Stdio::from(fs::File::create(&archive).unwrap()))
                .status()
                .unwrap()
        } else {
            Command::new("lzma")
                .args(["-c", hello.to_str().unwrap()])
                .stdout(std::process::Stdio::from(fs::File::create(&archive).unwrap()))
                .status()
                .unwrap()
        };
        assert!(status.success());
        assert_eq!(detect(&archive), Some(Format::Lzma));
        let out = extract_ok(&archive);
        assert!(find_named(&out, "hello").is_some() || find_named(&out, "hello.txt").is_some());
    }
    if has("mksquashfs") {
        let folder = dir.path().join("sq");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("hello.txt"), b"hello elavo-archive-utility\n").unwrap();
        let archive = dir.path().join("disk.squashfs");
        assert!(Command::new("mksquashfs")
            .args([
                folder.to_str().unwrap(),
                archive.to_str().unwrap(),
                "-noappend",
                "-quiet",
            ])
            .status()
            .unwrap()
            .success());
        assert_eq!(detect(&archive), Some(Format::SquashFs));
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
    }
    // An installer-shaped name whose bytes are a zip. innoextract declines
    // and 7-Zip opens it.
    let exe = dir.path().join("setup.exe");
    compress(
        &CompressRequest {
            inputs: vec![hello.clone()],
            output: exe.clone(),
            format: Format::Zip,
            level: Level::Store,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(detect(&exe), Some(Format::Exe));
    let out = extract_ok(&exe);
    assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
}

#[test]
fn zip_packages_and_an_unknown_tar_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let hello = dir.path().join("hello.txt");
    fs::write(&hello, b"hello elavo-archive-utility\n").unwrap();
    let zip = dir.path().join("app.zip");
    compress(
        &CompressRequest {
            inputs: vec![hello.clone()],
            output: zip.clone(),
            format: Format::Zip,
            level: Level::Store,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    for ext in ["apk", "jar", "ipa"] {
        let archive = dir.path().join(format!("app.{ext}"));
        fs::copy(&zip, &archive).unwrap();
        assert_eq!(
            detect(&archive),
            Some(match ext {
                "apk" => Format::Apk,
                "jar" => Format::Jar,
                _ => Format::Ipa,
            })
        );
        let out = extract_ok(&archive);
        assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
    }

    // A v7 tar has no ustar marker, so detection does not claim it. Extraction
    // still tries 7-Zip before failing.
    let archive = dir.path().join("blob.bin");
    assert!(Command::new("tar")
        .args([
            "--format=v7",
            "-cf",
            archive.to_str().unwrap(),
            "-C",
            dir.path().to_str().unwrap(),
            "hello.txt",
        ])
        .status()
        .unwrap()
        .success());
    assert_eq!(detect(&archive), None);
    let out = extract_ok(&archive);
    assert_eq!(find_named(&out, "hello.txt").unwrap(), b"hello elavo-archive-utility\n");
}

/// A ustar member whose name is exactly the bytes given, including `..`.
fn ustar_with_name(name: &[u8], data: &[u8]) -> Vec<u8> {
    fn octal(slot: &mut [u8], value: u64) {
        let text = format!("{:o}", value);
        let len = slot.len();
        assert!(text.len() < len);
        let start = len - 1 - text.len();
        slot[start..len - 1].copy_from_slice(text.as_bytes());
        slot[len - 1] = 0;
    }
    let mut header = vec![0u8; 512];
    assert!(name.len() < 100);
    header[..name.len()].copy_from_slice(name);
    octal(&mut header[100..108], 0o644);
    octal(&mut header[124..136], data.len() as u64);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    let checksum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(checksum.as_bytes());
    let mut out = header;
    out.extend_from_slice(data);
    out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
    out.extend(std::iter::repeat(0).take(1024));
    out
}

/// A stored RAR 4 archive. The bytes follow the published block layout
/// (marker, archive header, file header, payload) so the test does not
/// need the non-free `rar` tool.
fn stored_rar(name: &[u8], data: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
    fn header(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + body.len());
        out.extend_from_slice(&((crc32(body) & 0xffff) as u16).to_le_bytes());
        out.extend_from_slice(body);
        out
    }
    let marker = vec![0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];
    let mut archive_body = Vec::new();
    archive_body.push(0x73);
    archive_body.extend_from_slice(&0u16.to_le_bytes());
    archive_body.extend_from_slice(&13u16.to_le_bytes());
    archive_body.extend_from_slice(&0u16.to_le_bytes());
    archive_body.extend_from_slice(&0u32.to_le_bytes());
    let head_size = 32 + name.len() as u16;
    let mut file_body = Vec::new();
    file_body.push(0x74);
    file_body.extend_from_slice(&0u16.to_le_bytes());
    file_body.extend_from_slice(&head_size.to_le_bytes());
    file_body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    file_body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    file_body.push(0);
    file_body.extend_from_slice(&crc32(data).to_le_bytes());
    file_body.extend_from_slice(&0u32.to_le_bytes());
    file_body.push(20);
    file_body.push(0x30);
    file_body.extend_from_slice(&(name.len() as u16).to_le_bytes());
    file_body.extend_from_slice(&0x20u32.to_le_bytes());
    file_body.extend_from_slice(name);
    let mut end_body = Vec::new();
    end_body.push(0x7B);
    end_body.extend_from_slice(&0u16.to_le_bytes());
    end_body.extend_from_slice(&7u16.to_le_bytes());
    let mut bytes = marker;
    bytes.extend(header(&archive_body));
    bytes.extend(header(&file_body));
    bytes.extend_from_slice(data);
    bytes.extend(header(&end_body));
    bytes
}

fn fill_pattern(path: &Path, len: u64) {
    let mut file = File::create(path).unwrap();
    let mut left = len;
    let mut buf = vec![0u8; 256 * 1024];
    let mut state = 0xC0FFEE_u64;
    while left > 0 {
        for byte in &mut buf {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *byte = (state >> 33) as u8;
        }
        let n = (left as usize).min(buf.len());
        file.write_all(&buf[..n]).unwrap();
        left -= n as u64;
    }
}

fn files_match(left: &Path, right: &Path) {
    let mut a = File::open(left).unwrap();
    let mut b = File::open(right).unwrap();
    assert_eq!(
        a.metadata().unwrap().len(),
        b.metadata().unwrap().len(),
        "length"
    );
    let mut left_buf = [0u8; 64 * 1024];
    let mut right_buf = [0u8; 64 * 1024];
    loop {
        let left_n = a.read(&mut left_buf).unwrap();
        let right_n = b.read(&mut right_buf).unwrap();
        assert_eq!(left_n, right_n);
        if left_n == 0 {
            break;
        }
        assert_eq!(&left_buf[..left_n], &right_buf[..right_n]);
    }
}

/// Split `len` bytes into volumes of `part` bytes and extract the first volume.
fn split_round_trip(format: Format, password: Option<&str>, part: u64, len: u64) -> Vec<(String, u64)> {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("payload.bin");
    fill_pattern(&src, len);
    let archive = dir.path().join(format!("payload.{}", format.extension()));
    let outcome = compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: archive,
            format,
            level: Level::Store,
            password: password.map(str::to_string),
            split: Some(part),
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("compress {format:?}: {err}"));
    let mut parts: Vec<(String, u64, PathBuf)> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if name.starts_with("payload.") && name != "payload.bin" {
                Some((name, path.metadata().unwrap().len(), path))
            } else {
                None
            }
        })
        .collect();
    parts.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(parts.len() >= 2, "expected several volumes, got {parts:?}");
    assert_eq!(outcome.path, parts[0].2, "extract starts at the first volume");
    for (name, size, _) in &parts[..parts.len() - 1] {
        assert_eq!(*size, part, "{name} should be exactly {part} bytes");
    }
    assert!(parts.last().unwrap().1 < part, "the last volume is the remainder");
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive: outcome.path,
            destination: dest.clone(),
            password: password.map(str::to_string),
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("extract {format:?}: {err}"));
    files_match(&src, &dest.join("payload.bin"));
    parts
        .into_iter()
        .map(|(name, size, _)| (name, size))
        .collect()
}

#[test]
fn five_megabyte_splits_round_trip_for_7z_and_zip() {
    let part = 5 * 1024 * 1024;
    let len = 12 * 1024 * 1024;
    for format in [Format::SevenZ, Format::Zip] {
        let parts = split_round_trip(format, None, part, len);
        let names: Vec<_> = parts.iter().map(|(name, _)| name.as_str()).collect();
        eprintln!("{format:?} parts: {parts:?}");
        let split_names = names.iter().all(|name| {
            name.contains(".00") || name.contains(".z0") || name.ends_with(".zip")
        });
        assert!(split_names, "volume names, got {names:?}");
        assert!(
            names[0].ends_with(".001") || names[0].ends_with(".z01"),
            "{names:?}"
        );
    }
}

#[test]
fn split_with_a_password_round_trips() {
    let part = 5 * 1024 * 1024;
    let len = 12 * 1024 * 1024;
    for format in [Format::SevenZ, Format::Zip] {
        let parts = split_round_trip(format, Some("correct horse"), part, len);
        eprintln!("{format:?} password parts: {parts:?}");
        assert!(
            parts[0].0.ends_with(".001") || parts[0].0.ends_with(".z01"),
            "{:?}",
            parts[0]
        );
    }
}

#[test]
fn an_empty_split_is_a_single_archive() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("note.txt");
    fs::write(&src, b"one piece\n").unwrap();
    let archive = dir.path().join("note.7z");
    let outcome = compress(
        &CompressRequest {
            inputs: vec![src],
            output: archive.clone(),
            format: Format::SevenZ,
            level: Level::Store,
            split: None,
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(outcome.path, archive);
    assert!(archive.is_file());
    assert!(!dir.path().join("note.7z.001").is_file());
}

#[test]
fn formats_that_cannot_split_are_refused() {
    assert!(Format::SevenZ.supports_split());
    assert!(Format::Zip.supports_split());
    for format in [Format::Gz, Format::Tar, Format::Zst, Format::Xz, Format::Bz2] {
        assert!(!format.supports_split(), "{format:?}");
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("note.txt");
    fs::write(&src, b"plain\n").unwrap();
    let err = compress(
        &CompressRequest {
            inputs: vec![src],
            output: dir.path().join("note.gz"),
            format: Format::Gz,
            level: Level::Store,
            split: Some(1024),
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_err();
    match err {
        Error::Failed(message) => {
            assert!(
                message.contains("not split"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected a refusal, got {other}"),
    }
}

#[test]
fn one_gib_split_writes_gibibyte_volumes() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("wide.bin");
    let file = File::create(&src).unwrap();
    let len = 2_200 * 1024 * 1024;
    file.set_len(len).unwrap();
    let part = 1024 * 1024 * 1024;
    let outcome = compress(
        &CompressRequest {
            inputs: vec![src.clone()],
            output: dir.path().join("wide.7z"),
            format: Format::SevenZ,
            level: Level::Store,
            split: Some(part),
            ..CompressRequest::default()
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap_or_else(|err| panic!("{err}"));
    let mut parts: Vec<(String, u64, PathBuf)> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if name.starts_with("wide.7z") {
                Some((name, path.metadata().unwrap().len(), path))
            } else {
                None
            }
        })
        .collect();
    parts.sort_by(|left, right| left.0.cmp(&right.0));
    eprintln!("1 GiB parts: {parts:?}");
    assert!(parts.len() >= 3, "{parts:?}");
    assert_eq!(outcome.path, parts[0].2);
    for (name, size, _) in &parts[..parts.len() - 1] {
        assert_eq!(*size, part, "{name}");
    }
    let dest = dir.path().join("back");
    fs::create_dir(&dest).unwrap();
    extract(
        &ExtractRequest {
            archive: outcome.path,
            destination: dest.clone(),
            password: None,
            new_folder: NewFolder::Never,
            conflict: Conflict::Replace,
        },
        &Cancel::new(),
        |_| {},
    )
    .unwrap();
    let restored = dest.join("wide.bin");
    assert_eq!(fs::metadata(&restored).unwrap().len(), len);
    let mut restored_file = File::open(&restored).unwrap();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = restored_file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        assert!(buf[..n].iter().all(|byte| *byte == 0));
    }
}
