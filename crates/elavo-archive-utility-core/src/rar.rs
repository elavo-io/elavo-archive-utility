use std::path::Path;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::process::{self, Spec};
use crate::progress::{report, Progress};
use crate::settings::Conflict;
use crate::tools;
use crate::transplant;

/// Extract a RAR archive. Creation is intentionally unsupported: the RAR
/// format's compressor is not available under a free license. Extraction
/// uses `unar` (LGPL) or, for archives without a password, `unrar-free` (GPL)
/// as a separate program. 7-Zip is not used, so its non-free unRAR code is
/// never involved.
pub fn extract(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    conflict: Conflict,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let password = password.filter(|password| !password.is_empty());
    let found = tools::tools();
    if password.is_none() && crate::unarr::available() {
        match crate::unarr::extract(archive, dest, conflict, cancel) {
            Ok(files) => {
                report(&mut progress, Some(1.0), "Done");
                return Ok(files);
            }
            Err(Error::UnsafePath(path)) => return Err(Error::UnsafePath(path)),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => {}
        }
    }
    if let Some(unar) = found.unar.clone() {
        return run_unar(
            &unar,
            archive,
            dest,
            password,
            conflict,
            cancel,
            &mut progress,
        );
    }
    if password.is_some() {
        return Err(Error::ToolMissing {
            tool: "unar",
            purpose: "extract a password-protected RAR archive".into(),
        });
    }
    if let Some(unrar) = found.unrar_free.clone() {
        return run_unrar_free(&unrar, archive, dest, conflict, cancel, &mut progress);
    }
    Err(Error::ToolMissing {
        tool: "unar",
        purpose: "extract RAR archives".into(),
    })
}

pub fn list(archive: &Path, password: Option<&str>, cancel: &Cancel) -> Result<Vec<String>> {
    let password = password.filter(|password| !password.is_empty());
    let Some(lsar) = tools::tools().lsar.clone() else {
        return Ok(Vec::new());
    };
    let mut args = vec!["-json".into()];
    args.push(archive.display().to_string());
    let ran = if let Some(password) = password {
        process::run_on_pty(
            Spec {
                program: lsar,
                args,
                current_dir: None,
                writable: None,
                stdin: None,
                stdout_path: None,
            },
            password,
            cancel,
        )?
    } else {
        process::run(
            Spec {
                program: lsar,
                args,
                current_dir: None,
                writable: None,
                stdin: None,
                stdout_path: None,
            },
            cancel,
            |_| {},
        )?
    };
    if ran.code != Some(0) {
        if let Some(err) = process::password_problem(&ran.text, password.is_some()) {
            return Err(err);
        }
        if ran.text.to_lowercase().contains("password") {
            return Err(if password.is_some() {
                Error::WrongPassword
            } else {
                Error::PasswordRequired
            });
        }
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    Ok(names_from_lsar(&ran.text))
}

fn run_unar(
    unar: &Path,
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    conflict: Conflict,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    let work = tempfile::tempdir().map_err(Error::from)?;
    let label = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive.rar".into());
    report(progress, None, &label);
    let mut args = vec![
        "-q".into(),
        "-D".into(),
        "-k".into(),
        "skip".into(),
        "-o".into(),
        work.path().display().to_string(),
    ];
    args.push(archive.display().to_string());
    let spec = Spec {
        program: unar.to_path_buf(),
        args,
        current_dir: Some(work.path().to_path_buf()),
        writable: Some(work.path().to_path_buf()),
        stdin: None,
        stdout_path: None,
    };
    let ran = if let Some(password) = password {
        process::run_on_pty(spec, password, cancel)?
    } else {
        process::run(spec, cancel, |_| {})?
    };
    if ran.code != Some(0) {
        let lower = ran.text.to_lowercase();
        if lower.contains("password") {
            return Err(if password.is_some() {
                Error::WrongPassword
            } else {
                Error::PasswordRequired
            });
        }
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    let files = transplant::transplant(work.path(), dest, conflict, cancel)?;
    report(progress, Some(1.0), "Done");
    Ok(files)
}

fn run_unrar_free(
    unrar: &Path,
    archive: &Path,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    let work = tempfile::tempdir().map_err(Error::from)?;
    let ran = process::run(
        Spec {
            program: unrar.to_path_buf(),
            args: vec![
                "-x".into(),
                archive.display().to_string(),
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
    let files = transplant::transplant(work.path(), dest, conflict, cancel)?;
    report(progress, Some(1.0), "Done");
    Ok(files)
}

fn names_from_lsar(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("\"XADFileName\"") else {
            continue;
        };
        let Some(start) = rest.find('"') else {
            continue;
        };
        let rest = &rest[start + 1..];
        if let Some(end) = rest.find('"') {
            names.push(unescape(&rest[..end]));
        }
    }
    names
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
