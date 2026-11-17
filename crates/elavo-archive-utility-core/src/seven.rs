use std::fs;
use std::path::{Path, PathBuf};

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::format::{Format, Level};
use crate::names::{common_parent, top_names};
use crate::process::{self, Spec};
use crate::progress::{report, Progress};
use crate::settings::Conflict;
use crate::tools::{self, Tools};
use crate::transplant;

// The arguments are the archive request's fields, passed through as is.
#[allow(clippy::too_many_arguments)]
pub fn compress(
    inputs: &[PathBuf],
    output: &Path,
    format: Format,
    level: Level,
    password: Option<&str>,
    encrypt_names: bool,
    aes256: bool,
    split: Option<u64>,
    exclude_macos: bool,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let tools = tools::tools();
    let password = blank_as_none(password);
    // 7-Zip 23 pre-creates ZIP volumes and then fails with "File exists"
    // when it opens the first one again. Info-ZIP writes name.z01 … and
    // name.zip, which 7-Zip can extract starting at the first part.
    if format == Format::Zip && split.is_some() && !aes256 {
        if let Some(zip) = tools.zip.clone() {
            return compress_zip_split(
                &zip,
                inputs,
                output,
                level,
                password,
                split.unwrap_or(0),
                exclude_macos,
                cancel,
                &mut progress,
            );
        }
    }
    if tools.seven_zip.is_none() {
        if password.is_some() || split.is_some() || format == Format::SevenZ {
            return Err(missing(format));
        }
        if format == Format::Zip {
            if let Some(zip) = tools.zip.clone() {
                return compress_zip_cli(
                    &zip,
                    inputs,
                    output,
                    level,
                    exclude_macos,
                    cancel,
                    &mut progress,
                );
            }
        }
        return Err(missing(format));
    }

    let staged = stage(inputs, exclude_macos)?;
    let work = tempfile::tempdir().map_err(Error::from)?;
    // 7-Zip volume temps collide with the archive when -w is the same folder.
    let scratch = work.path().join("scratch");
    fs::create_dir(&scratch)?;
    let leaf = format!("out.{}", format.extension());
    let archive = work.path().join(&leaf);
    let names: Vec<String> = staged
        .names
        .iter()
        .map(|name| format!("./{name}"))
        .collect();
    let args = compress_args(
        format,
        level,
        password,
        encrypt_names && password.is_some(),
        aes256,
        split,
        &scratch,
        &archive,
        &names,
    );
    let secret = password.map(|password| format!("{password}\n").into_bytes());
    let label = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| leaf.clone());
    let ran = run_tool(
        tools.seven_zip.clone().unwrap(),
        args,
        &staged.cwd,
        work.path(),
        secret,
        cancel,
        &mut progress,
        &label,
    )?;
    check(&ran, password.is_some())?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    move_produced(work.path(), &leaf, output)?;
    report(&mut progress, Some(1.0), label);
    Ok(inputs.len() as u64)
}

// The arguments are the archive request's fields, passed through as is.
#[allow(clippy::too_many_arguments)]
fn compress_zip_split(
    zip: &Path,
    inputs: &[PathBuf],
    output: &Path,
    level: Level,
    password: Option<&str>,
    split: u64,
    exclude_macos: bool,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    let staged = stage(inputs, exclude_macos)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut args = vec![
        "-s".into(),
        split.to_string(),
        level.zip_cli().into(),
        output.display().to_string(),
    ];
    if password.is_some() {
        args.insert(0, "-e".into());
    }
    args.extend(staged.names.iter().cloned());
    let spec = process::Spec {
        program: zip.to_path_buf(),
        args,
        current_dir: Some(staged.cwd.clone()),
        writable: output.parent().map(Path::to_path_buf),
        stdin: None,
        stdout_path: None,
    };
    let ran = if let Some(password) = password {
        process::run_on_pty_times(spec, password, 2, cancel)?
    } else {
        process::run(spec, cancel, |_| {})?
    };
    if ran.code != Some(0) {
        if let Some(err) = process::password_problem(&ran.text, password.is_some()) {
            return Err(err);
        }
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    report(progress, Some(1.0), "Done");
    Ok(staged.names.len() as u64)
}

pub fn extract(
    archive: &Path,
    dest: &Path,
    format: Format,
    password: Option<&str>,
    conflict: Conflict,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let tools = tools::tools();
    let password = blank_as_none(password);
    fs::create_dir_all(dest)?;
    if tools.seven_zip.is_none() {
        return extract_fallback(
            tools,
            archive,
            dest,
            format,
            password,
            conflict,
            cancel,
            &mut progress,
        );
    }
    let work = tempfile::tempdir().map_err(Error::from)?;
    let mut args = vec![
        "x".into(),
        "-y".into(),
        "-spd".into(),
        "-bso1".into(),
        "-bse1".into(),
        "-bsp1".into(),
        format!("-o{}", work.path().display()),
        format!("-w{}", work.path().display()),
    ];
    let secret = password_bytes(password);
    args.push("--".into());
    args.push(archive.display().to_string());
    let label = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".into());
    let ran = run_tool(
        tools.seven_zip.clone().unwrap(),
        args,
        work.path(),
        work.path(),
        secret,
        cancel,
        &mut progress,
        &label,
    )?;
    check(&ran, password.is_some())?;
    let files = transplant::transplant(work.path(), dest, conflict, cancel)?;
    report(&mut progress, Some(1.0), "Done");
    Ok(files)
}

/// Unpack an ar archive (a .deb's outer layer) as is. 7-Zip would otherwise
/// open a .deb straight to its data tarball and drop the control files.
pub fn unpack_ar(archive: &Path, dest: &Path, cancel: &Cancel) -> Result<()> {
    let tools = tools::tools();
    let (program, args) = if let Some(seven) = tools.seven_zip.clone() {
        (
            seven,
            vec![
                "x".into(),
                "-y".into(),
                "-tar".into(),
                "-bso1".into(),
                "-bse1".into(),
                format!("-o{}", dest.display()),
                "--".into(),
                archive.display().to_string(),
            ],
        )
    } else if let Some(bsdtar) = tools.bsdtar.clone() {
        (
            bsdtar,
            vec![
                "-x".into(),
                "-f".into(),
                archive.display().to_string(),
                "-C".into(),
                dest.display().to_string(),
            ],
        )
    } else {
        return Err(Error::ToolMissing {
            tool: "7z",
            purpose: "open this package".into(),
        });
    };
    let ran = process::run(
        Spec {
            program,
            args,
            current_dir: Some(dest.to_path_buf()),
            writable: Some(dest.to_path_buf()),
            stdin: None,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) {
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    Ok(())
}

pub fn list(archive: &Path, password: Option<&str>, cancel: &Cancel) -> Result<Vec<String>> {
    let tools = tools::tools();
    let password = blank_as_none(password);
    let Some(program) = tools.seven_zip.clone() else {
        return list_fallback(tools, archive, cancel);
    };
    let work = tempfile::tempdir().map_err(Error::from)?;
    let mut args = vec!["l".into(), "-ba".into(), "-slt".into(), "-y".into()];
    let secret = password_bytes(password);
    args.push("--".into());
    args.push(archive.display().to_string());
    let ran = process::run(
        Spec {
            program,
            args,
            current_dir: Some(work.path().to_path_buf()),
            writable: Some(work.path().to_path_buf()),
            stdin: secret,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) {
        if let Some(err) = process::password_problem(&ran.text, password.is_some()) {
            return Err(err);
        }
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    Ok(paths_from_slt(&ran.text))
}

// The arguments are the archive request's fields, passed through as is.
#[allow(clippy::too_many_arguments)]
fn compress_args(
    format: Format,
    level: Level,
    password: Option<&str>,
    encrypt_names: bool,
    aes256: bool,
    split: Option<u64>,
    work: &Path,
    archive: &Path,
    names: &[String],
) -> Vec<String> {
    let kind = match format {
        Format::Zip => "zip",
        Format::Wim => "wim",
        _ => "7z",
    };
    let mut args = vec![
        "a".into(),
        "-y".into(),
        "-spd".into(),
        "-bso1".into(),
        "-bse1".into(),
        "-bsp1".into(),
        format!("-t{kind}"),
        format!("-mx={}", level.seven()),
        format!("-w{}", work.display()),
    ];
    if password.is_some() {
        args.push("-p".into());
        if format == Format::Zip && aes256 {
            args.push("-mem=AES256".into());
        } else if format != Format::Zip && encrypt_names {
            args.push("-mhe=on".into());
        }
    }
    if let Some(bytes) = split {
        args.push(process::volume_switch(bytes));
    }
    args.push("--".into());
    args.push(archive.display().to_string());
    args.extend(names.iter().cloned());
    args
}

struct Staged {
    cwd: PathBuf,
    names: Vec<String>,
    _keep: Option<tempfile::TempDir>,
}

fn stage(inputs: &[PathBuf], exclude_macos: bool) -> Result<Staged> {
    if let Some(parent) = common_parent(inputs) {
        let names: Vec<String> = inputs
            .iter()
            .map(|path| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "item".into())
            })
            .filter(|name| !(exclude_macos && crate::names::is_macos_metadata(name)))
            .collect();
        if names.is_empty() {
            return Err(Error::Failed(
                "Nothing left to archive after excluding Mac resource forks.".into(),
            ));
        }
        return Ok(Staged {
            cwd: parent,
            names,
            _keep: None,
        });
    }
    let temp = tempfile::tempdir().map_err(Error::from)?;
    let names = top_names(inputs);
    for (input, name) in inputs.iter().zip(names.iter()) {
        if exclude_macos && crate::names::is_macos_metadata(name) {
            continue;
        }
        mirror(input, &temp.path().join(name), exclude_macos, 0)?;
    }
    Ok(Staged {
        cwd: temp.path().to_path_buf(),
        names,
        _keep: Some(temp),
    })
}

fn mirror(src: &Path, dst: &Path, exclude_macos: bool, depth: u32) -> Result<()> {
    if depth > 32 {
        return Err(Error::Failed(format!(
            "Too many links while reading {}.",
            src.display()
        )));
    }
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        let target = fs::canonicalize(src)
            .map_err(|_| Error::Failed(format!("Couldn't follow the link {}.", src.display())))?;
        return mirror(&target, dst, exclude_macos, depth + 1);
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let name = entry.file_name();
            if exclude_macos && crate::names::is_macos_metadata(&name.to_string_lossy()) {
                continue;
            }
            mirror(&entry.path(), &dst.join(name), exclude_macos, depth)?;
        }
        return Ok(());
    }
    if meta.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::hard_link(src, dst) {
            Ok(()) => Ok(()),
            Err(_) => {
                fs::copy(src, dst)?;
                Ok(())
            }
        }
    } else {
        Err(Error::Failed(format!("Can't archive {}.", src.display())))
    }
}

fn password_bytes(password: Option<&str>) -> Option<Vec<u8>> {
    let password = password.filter(|password| !password.is_empty())?;
    // 7-Zip reads this line when it prints "Enter password". Passing `-p`
    // with no value on extract is an empty password, so extract, list, and
    // test omit `-p` and answer the prompt. Create still passes `-p` so the
    // archive is actually encrypted, and answers the same prompt.
    Some(format!("{password}\n").into_bytes())
}

pub fn test_archive(archive: &Path, password: Option<&str>, cancel: &Cancel) -> Result<()> {
    let Some(program) = tools::tools().seven_zip.clone() else {
        return Err(Error::ToolMissing {
            tool: "7z",
            purpose: "test this archive".into(),
        });
    };
    let work = tempfile::tempdir().map_err(Error::from)?;
    let mut args = vec!["t".into(), "-y".into(), "-bso1".into(), "-bse1".into()];
    let secret = password_bytes(password);
    args.push("--".into());
    args.push(archive.display().to_string());
    let ran = process::run(
        Spec {
            program,
            args,
            current_dir: Some(work.path().to_path_buf()),
            writable: Some(work.path().to_path_buf()),
            stdin: secret,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    check(&ran, password.is_some())
}

// The arguments are the archive request's fields, passed through as is.
#[allow(clippy::too_many_arguments)]
fn run_tool(
    program: PathBuf,
    args: Vec<String>,
    cwd: &Path,
    writable: &Path,
    stdin: Option<Vec<u8>>,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
    label: &str,
) -> Result<process::Output> {
    let mut logical = Vec::new();
    let mut last = None;
    process::run(
        Spec {
            program,
            args,
            current_dir: Some(cwd.to_path_buf()),
            writable: Some(writable.to_path_buf()),
            stdin,
            stdout_path: None,
        },
        cancel,
        |chunk| {
            process::apply_controls(&mut logical, chunk);
            let text = String::from_utf8_lossy(&logical);
            if let Some(pct) = process::last_percent(&text) {
                if last != Some(pct) {
                    last = Some(pct);
                    report(progress, Some(pct as f64 / 100.0), label.to_string());
                }
            }
        },
    )
}

fn check(output: &process::Output, supplied: bool) -> Result<()> {
    if output.code != Some(0) {
        if let Some(err) = process::password_problem(&output.text, supplied) {
            return Err(err);
        }
    }
    match output.code {
        Some(0) | Some(1) => Ok(()),
        _ => Err(Error::Failed(process::summarize(&output.text))),
    }
}

fn move_produced(dir: &Path, prefix: &str, final_path: &Path) -> Result<PathBuf> {
    let mut produced: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| is_volume(name, prefix))
        })
        .collect();
    produced.sort();
    if produced.is_empty() {
        return Err(Error::Failed(
            "The archive tool did not write a file.".into(),
        ));
    }
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let final_name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| prefix.to_string());
    let mut first = None;
    for src in &produced {
        let src_name = src.file_name().unwrap().to_string_lossy();
        let suffix = src_name.strip_prefix(prefix).unwrap_or("");
        let dest = parent.join(format!("{final_name}{suffix}"));
        move_file(src, &dest)?;
        if first.is_none() {
            first = Some(dest);
        }
    }
    Ok(first.unwrap())
}

fn is_volume(name: &str, prefix: &str) -> bool {
    if name == prefix {
        return true;
    }
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some(num) = rest.strip_prefix('.') else {
        return false;
    };
    num.len() == 3 && num.chars().all(|c| c.is_ascii_digit())
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            fs::copy(from, to)?;
            fs::remove_file(from)?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

fn paths_from_slt(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("Path = "))
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect()
}

fn blank_as_none(password: Option<&str>) -> Option<&str> {
    password.filter(|password| !password.is_empty())
}

fn missing(format: Format) -> Error {
    Error::ToolMissing {
        tool: "7z",
        purpose: format!("create and extract {} archives", format.label()),
    }
}

// The arguments are the archive request's fields, passed through as is.
#[allow(clippy::too_many_arguments)]
fn extract_fallback(
    tools: &Tools,
    archive: &Path,
    dest: &Path,
    format: Format,
    password: Option<&str>,
    conflict: Conflict,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    if password.is_some() {
        return Err(missing(format));
    }
    let work = tempfile::tempdir().map_err(Error::from)?;
    if matches!(
        format,
        Format::Zip | Format::Apk | Format::Jar | Format::Ipa
    ) {
        if let Some(unzip) = tools.unzip.clone() {
            let ran = process::run(
                Spec {
                    program: unzip,
                    args: vec![
                        "-q".into(),
                        archive.display().to_string(),
                        "-d".into(),
                        work.path().display().to_string(),
                    ],
                    current_dir: Some(work.path().to_path_buf()),
                    writable: Some(work.path().to_path_buf()),
                    stdin: None,
                    stdout_path: None,
                },
                cancel,
                |_| {},
            )?;
            if ran.code != Some(0) {
                return Err(Error::Failed(process::summarize(&ran.text)));
            }
            return transplant::transplant(work.path(), dest, conflict, cancel);
        }
    }
    if matches!(
        format,
        Format::Iso
            | Format::Zip
            | Format::Apk
            | Format::Jar
            | Format::Ipa
            | Format::SevenZ
            | Format::Ar
            | Format::Deb
            | Format::Cpio
            | Format::Xar
            | Format::Tar
    ) {
        if let Some(bsdtar) = tools.bsdtar.clone() {
            let ran = process::run(
                Spec {
                    program: bsdtar,
                    args: vec![
                        "-x".into(),
                        "-f".into(),
                        archive.display().to_string(),
                        "-C".into(),
                        work.path().display().to_string(),
                    ],
                    current_dir: Some(work.path().to_path_buf()),
                    writable: Some(work.path().to_path_buf()),
                    stdin: None,
                    stdout_path: None,
                },
                cancel,
                |_| {},
            )?;
            if ran.code != Some(0) {
                return Err(Error::Failed(process::summarize(&ran.text)));
            }
            report(progress, Some(1.0), "Done");
            return transplant::transplant(work.path(), dest, conflict, cancel);
        }
    }
    if format == Format::Cab {
        if let Some(cabextract) = tools.cabextract.clone() {
            let ran = process::run(
                Spec {
                    program: cabextract,
                    args: vec![
                        "-q".into(),
                        "-d".into(),
                        work.path().display().to_string(),
                        archive.display().to_string(),
                    ],
                    current_dir: Some(work.path().to_path_buf()),
                    writable: Some(work.path().to_path_buf()),
                    stdin: None,
                    stdout_path: None,
                },
                cancel,
                |_| {},
            )?;
            if ran.code != Some(0) {
                return Err(Error::Failed(process::summarize(&ran.text)));
            }
            return transplant::transplant(work.path(), dest, conflict, cancel);
        }
    }
    Err(Error::ToolMissing {
        tool: format.extract_tool(),
        purpose: "open this".into(),
    })
}

fn list_fallback(tools: &Tools, archive: &Path, cancel: &Cancel) -> Result<Vec<String>> {
    let Some(bsdtar) = tools.bsdtar.clone() else {
        return Err(Error::ToolMissing {
            tool: "7z",
            purpose: "read this archive".into(),
        });
    };
    let ran = process::run(
        Spec {
            program: bsdtar,
            args: vec!["-tf".into(), archive.display().to_string()],
            current_dir: None,
            writable: None,
            stdin: None,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) {
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    Ok(ran
        .text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

fn compress_zip_cli(
    zip: &Path,
    inputs: &[PathBuf],
    output: &Path,
    level: Level,
    exclude_macos: bool,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    let staged = stage(inputs, exclude_macos)?;
    let names: Vec<String> = staged
        .names
        .iter()
        .map(|name| format!("./{name}"))
        .collect();
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut args = vec![
        "-r".into(),
        "-q".into(),
        level.zip_cli().into(),
        output.display().to_string(),
    ];
    args.extend(names);
    let ran = process::run(
        Spec {
            program: zip.to_path_buf(),
            args,
            current_dir: Some(staged.cwd),
            writable: output.parent().map(Path::to_path_buf),
            stdin: None,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) {
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    report(progress, Some(1.0), "Done");
    Ok(inputs.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_names_match_the_archive_prefix() {
        assert!(is_volume("out.7z", "out.7z"));
        assert!(is_volume("out.7z.001", "out.7z"));
        assert!(!is_volume("out.7z.tmp", "out.7z"));
        let _ = Level::Fast;
    }
}
