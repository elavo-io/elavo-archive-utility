use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::cancel::Cancel;
use crate::error::{cancelled_io, Error, Result};
use crate::names::is_safe_rel;
use crate::settings::Conflict;

/// Copy an extracted tree into `dest`, refusing to follow a symlink out of it.
pub fn transplant(from: &Path, dest: &Path, conflict: Conflict, cancel: &Cancel) -> Result<u64> {
    fs::create_dir_all(dest)?;
    let mut files = 0u64;
    copy_tree(from, dest, Path::new(""), conflict, cancel, &mut files)?;
    Ok(files)
}

fn copy_tree(
    from_root: &Path,
    dest_root: &Path,
    rel: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    files: &mut u64,
) -> Result<()> {
    cancel.check()?;
    let src = from_root.join(rel);
    let meta = fs::symlink_metadata(&src)?;
    let dest = if rel.as_os_str().is_empty() {
        dest_root.to_path_buf()
    } else {
        checked_join(dest_root, rel)?
    };
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        let Some(dest) = place_leaf(&dest, conflict)? else {
            return Ok(());
        };
        if let Some(parent) = dest.parent() {
            prepare_dirs(
                dest_root,
                parent.strip_prefix(dest_root).unwrap_or(Path::new("")),
            )?;
        }
        let target = fs::read_link(&src)?;
        let _ = fs::remove_file(&dest);
        std::os::unix::fs::symlink(&target, &dest)?;
        *files += 1;
        return Ok(());
    }
    if meta.is_dir() {
        if !rel.as_os_str().is_empty() {
            ensure_real_dir(&dest, meta.permissions().mode())?;
        }
        let mut children: Vec<_> = fs::read_dir(&src)?
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .map(|entry| entry.file_name())
            .collect();
        children.sort();
        for name in children {
            copy_tree(
                from_root,
                dest_root,
                &rel.join(name),
                conflict,
                cancel,
                files,
            )?;
        }
        return Ok(());
    }
    if meta.is_file() {
        let Some(dest) = place_leaf(&dest, conflict)? else {
            return Ok(());
        };
        if let Some(parent) = dest.parent() {
            let rel_parent = parent.strip_prefix(dest_root).unwrap_or(Path::new(""));
            prepare_dirs(dest_root, rel_parent)?;
        }
        copy_file(&src, &dest, meta.permissions().mode(), cancel)?;
        if let Ok(modified) = meta.modified() {
            if let Ok(file) = File::open(&dest) {
                let _ = file.set_modified(modified);
            }
        }
        *files += 1;
        return Ok(());
    }
    Ok(())
}

pub fn prepare_dirs(root: &Path, rel: &Path) -> Result<()> {
    if !rel.as_os_str().is_empty() && !is_safe_rel(rel) {
        return Err(Error::UnsafePath(rel.display().to_string()));
    }
    let mut cur = root.to_path_buf();
    fs::create_dir_all(root)?;
    for component in rel.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        cur.push(name);
        ensure_real_dir(&cur, 0o755)?;
    }
    Ok(())
}

pub fn checked_join(root: &Path, rel: &Path) -> Result<PathBuf> {
    if !is_safe_rel(rel) {
        return Err(Error::UnsafePath(rel.display().to_string()));
    }
    Ok(root.join(rel))
}

fn ensure_real_dir(path: &Path, mode: u32) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(Error::UnsafePath(path.display().to_string())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777));
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

fn place_leaf(dest: &Path, conflict: Conflict) -> Result<Option<PathBuf>> {
    if fs::symlink_metadata(dest).is_err() {
        return Ok(Some(dest.to_path_buf()));
    }
    match conflict {
        Conflict::Skip => Ok(None),
        Conflict::Replace => {
            match fs::symlink_metadata(dest) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                    return Err(Error::Failed(format!(
                        "{} is a folder, so it was left in place.",
                        dest.display()
                    )));
                }
                _ => {
                    let _ = fs::remove_file(dest);
                }
            }
            Ok(Some(dest.to_path_buf()))
        }
        Conflict::Rename => Ok(Some(unique_path(dest))),
    }
}

pub fn unique_path(path: &Path) -> PathBuf {
    if fs::symlink_metadata(path).is_err() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned());
    for n in 2..10_000 {
        let name = match &ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    path.to_path_buf()
}

pub fn copy_file(src: &Path, dest: &Path, mode: u32, cancel: &Cancel) -> Result<()> {
    let mut input = File::open(src)?;
    write_new(&mut input, dest, mode, cancel)?;
    Ok(())
}

pub fn write_new(input: &mut impl Read, dest: &Path, mode: u32, cancel: &Cancel) -> Result<u64> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode & 0o777)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dest)?;
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    let copied = (|| -> Result<u64> {
        loop {
            cancel.check().map_err(|_| cancelled_io())?;
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n])?;
            total += n as u64;
        }
        Ok(total)
    })();
    if copied.is_err() {
        // A cancelled or failed copy leaves no half-written file behind.
        drop(output);
        let _ = fs::remove_file(dest);
    }
    copied
}

pub fn set_mtime(path: &Path, secs: u64) {
    let when = UNIX_EPOCH + Duration::from_secs(secs);
    if let Ok(file) = File::open(path) {
        let _ = file.set_modified(when);
    }
}
