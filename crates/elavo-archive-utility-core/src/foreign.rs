//! Formats that are created or opened by an external program other than
//! the 7-Zip path in `seven`: ISO images, DMG images, and Windows installers.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::format::Format;
use crate::names::{common_parent, top_names};
use crate::process::{self, Spec};
use crate::progress::{report, Progress};
use crate::settings::Conflict;
use crate::seven;
use crate::tools::{self, IsoKind};

pub fn create_iso(
    inputs: &[PathBuf],
    output: &Path,
    exclude_macos: bool,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let tools = tools::tools();
    let program = tools
        .iso_creator
        .clone()
        .ok_or_else(|| Error::ToolMissing {
            tool: "xorriso",
            purpose: "create ISO images".into(),
        })?;
    let staged = stage_dir(inputs, exclude_macos)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let label = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "disc.iso".into());
    report(&mut progress, None, &label);
    let args = match tools.iso_kind {
        IsoKind::Xorriso => vec![
            "-as".into(),
            "mkisofs".into(),
            "-o".into(),
            output.display().to_string(),
            "-R".into(),
            "-J".into(),
            "-V".into(),
            "ARCHIVE".into(),
            staged.path.display().to_string(),
        ],
        IsoKind::Genisoimage => vec![
            "-o".into(),
            output.display().to_string(),
            "-R".into(),
            "-J".into(),
            "-V".into(),
            "ARCHIVE".into(),
            staged.path.display().to_string(),
        ],
        IsoKind::None => {
            return Err(Error::ToolMissing {
                tool: "xorriso",
                purpose: "create ISO images".into(),
            })
        }
    };
    let ran = process::run(
        Spec {
            program,
            args,
            current_dir: Some(staged.path.clone()),
            writable: output.parent().map(Path::to_path_buf),
            stdin: None,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) || !output.is_file() {
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    report(&mut progress, Some(1.0), label);
    Ok(inputs.len() as u64)
}

pub fn create_dmg(
    inputs: &[PathBuf],
    output: &Path,
    exclude_macos: bool,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let program = tools::tools()
        .dmg_build
        .clone()
        .ok_or_else(|| Error::ToolMissing {
            tool: "dmg",
            purpose: "create DMG images".into(),
        })?;
    let staged = stage_dir(inputs, exclude_macos)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let label = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "disk.dmg".into());
    report(&mut progress, None, &label);
    let ran = process::run(
        Spec {
            program,
            args: vec![
                "build".into(),
                staged.path.display().to_string(),
                output.display().to_string(),
            ],
            current_dir: Some(staged.path.clone()),
            writable: output.parent().map(Path::to_path_buf),
            stdin: None,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) || !output.is_file() {
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    report(&mut progress, Some(1.0), label);
    Ok(inputs.len() as u64)
}

/// Inno Setup via `innoextract` when it is installed, then 7-Zip for NSIS
/// and other self-extracting executables.
pub fn extract_exe(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    conflict: Conflict,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    if let Some(inno) = tools::tools().innoextract.clone() {
        if let Ok(files) = extract_inno(&inno, archive, dest, conflict, cancel, &mut progress) {
            return Ok(files);
        }
    }
    if tools::tools().seven_zip.is_some() {
        return seven::extract(
            archive,
            dest,
            Format::Exe,
            password,
            conflict,
            cancel,
            progress,
        );
    }
    Err(Error::ToolMissing {
        tool: "7z",
        purpose: "open this installer".into(),
    })
}

fn extract_inno(
    program: &Path,
    archive: &Path,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    let work = tempfile::tempdir().map_err(Error::from)?;
    let ran = process::run(
        Spec {
            program: program.to_path_buf(),
            args: vec![
                "--extract".into(),
                "--quiet".into(),
                "--output-dir".into(),
                work.path().display().to_string(),
                "--".into(),
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
    let files = crate::transplant::transplant(work.path(), dest, conflict, cancel)?;
    report(progress, Some(1.0), "Done");
    Ok(files)
}

struct Staged {
    path: PathBuf,
    _keep: Option<tempfile::TempDir>,
}

fn stage_dir(inputs: &[PathBuf], exclude_macos: bool) -> Result<Staged> {
    if inputs.len() == 1 && inputs[0].is_dir() && !exclude_macos {
        return Ok(Staged {
            path: inputs[0].clone(),
            _keep: None,
        });
    }
    if common_parent(inputs).is_some() {
        if inputs.len() == 1 && inputs[0].is_dir() {
            return Ok(Staged {
                path: inputs[0].clone(),
                _keep: None,
            });
        }
        let temp = tempfile::tempdir().map_err(Error::from)?;
        for input in inputs {
            let name = input
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "item".into());
            if exclude_macos && crate::names::is_macos_metadata(&name) {
                continue;
            }
            copy_entry(input, &temp.path().join(&name), exclude_macos, 0)?;
        }
        return Ok(Staged {
            path: temp.path().to_path_buf(),
            _keep: Some(temp),
        });
    }
    let temp = tempfile::tempdir().map_err(Error::from)?;
    let names = top_names(inputs);
    for (input, name) in inputs.iter().zip(names.iter()) {
        if exclude_macos && crate::names::is_macos_metadata(name) {
            continue;
        }
        copy_entry(input, &temp.path().join(name), exclude_macos, 0)?;
    }
    Ok(Staged {
        path: temp.path().to_path_buf(),
        _keep: Some(temp),
    })
}

fn copy_entry(src: &Path, dst: &Path, exclude_macos: bool, depth: u32) -> Result<()> {
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
        return copy_entry(&target, dst, exclude_macos, depth + 1);
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let name = entry.file_name();
            if exclude_macos && crate::names::is_macos_metadata(&name.to_string_lossy()) {
                continue;
            }
            copy_entry(&entry.path(), &dst.join(name), exclude_macos, depth)?;
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
