use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use bzip2::Compression as BzLevel;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression as GzLevel;
use tar::{Builder, EntryType, Header};
use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

use crate::cancel::Cancel;
use crate::error::{cancelled_io, Error, Result};
use crate::format::{Format, Level};
use crate::names::{is_safe_rel, top_names};
use crate::progress::{report, Progress};
use crate::settings::Conflict;
use crate::transplant::{self, checked_join, prepare_dirs, unique_path};

pub fn compress_tree(
    inputs: &[PathBuf],
    output: &Path,
    format: Format,
    level: Level,
    exclude_macos: bool,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    cancel.check()?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = partial_path(output);
    let _guard = DeleteOnDrop(&partial);
    let file = File::create(&partial)?;
    let out = Out::create(format, level, file)?;
    let mut total = 0u64;
    for input in inputs {
        measure(input, &mut total)?;
    }
    let total = total.max(1);
    let mut done = 0u64;
    let mut files = 0u64;
    let names = top_names(inputs);
    if format.is_stream() {
        return Err(Error::Failed(
            "Internal error: a stream format is not a tar archive.".into(),
        ));
    }
    let mut builder = Builder::new(out);
    for (input, name) in inputs.iter().zip(names.iter()) {
        if exclude_macos && crate::names::is_macos_metadata(name) {
            continue;
        }
        files += append_path(
            &mut builder,
            input,
            name,
            exclude_macos,
            cancel,
            &mut progress,
            &mut done,
            total,
        )?;
    }
    let out = builder.into_inner()?;
    out.finish()?;
    fs::rename(&partial, output)?;
    report(&mut progress, Some(1.0), output.display().to_string());
    Ok(files)
}

pub fn compress_stream(
    input: &Path,
    output: &Path,
    format: Format,
    level: Level,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    cancel.check()?;
    let meta = fs::metadata(input)?;
    if !meta.is_file() {
        return Err(Error::Failed(format!(
            "{} compresses one file. Choose {} for a folder.",
            format.label(),
            format.tar_equivalent().label()
        )));
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = partial_path(output);
    let _guard = DeleteOnDrop(&partial);
    let infile = File::open(input)?;
    let outfile = File::create(&partial)?;
    let mut out = Out::create(format, level, outfile)?;
    let total = meta.len().max(1);
    let mut reader = ProgressRead {
        inner: infile,
        cancel,
        done: 0,
        total,
        step: 0,
        label: input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into()),
        progress: &mut progress,
    };
    io::copy(&mut reader, &mut out)?;
    out.finish()?;
    fs::rename(&partial, output)?;
    report(&mut progress, Some(1.0), output.display().to_string());
    Ok(1)
}

pub fn list_tar(archive: &Path, format: Format, cancel: &Cancel) -> Result<Vec<String>> {
    let file = File::open(archive)?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let counted = CountRead {
        inner: file,
        done: Arc::new(AtomicU64::new(0)),
        cancel: cancel.clone(),
        total,
    };
    let decoder = open_decoder(format, counted)?;
    let mut tar = tar::Archive::new(decoder);
    let mut names = Vec::new();
    for entry in tar.entries()? {
        cancel.check()?;
        let entry = entry?;
        let path = entry.path()?;
        names.push(path.to_string_lossy().replace('\\', "/"));
    }
    Ok(names)
}

/// Tar members are written straight into `dest`. If the extraction fails or
/// is cancelled, the files it wrote and the top-level folders it created are
/// removed again, so nothing half-extracted is left beside the archive.
/// Entries that were already there, or that something else creates
/// meanwhile, are left alone.
pub fn extract_tar(
    archive: &Path,
    format: Format,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    progress: impl FnMut(Progress),
) -> Result<u64> {
    fs::create_dir_all(dest)?;
    let mut created = Vec::new();
    let result = extract_tar_into(
        archive,
        format,
        dest,
        conflict,
        cancel,
        progress,
        &mut created,
    );
    if result.is_err() {
        for path in created.iter().rev() {
            let is_dir = fs::symlink_metadata(path)
                .map(|meta| meta.is_dir() && !meta.file_type().is_symlink())
                .unwrap_or(false);
            let _ = if is_dir {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
        }
    }
    result
}

// The arguments are extract_tar's, plus the list of files written so far.
#[allow(clippy::too_many_arguments)]
fn extract_tar_into(
    archive: &Path,
    format: Format,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
    created: &mut Vec<PathBuf>,
) -> Result<u64> {
    let file = File::open(archive)?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(1).max(1);
    let done = Arc::new(AtomicU64::new(0));
    let counted = CountRead {
        inner: file,
        done: done.clone(),
        cancel: cancel.clone(),
        total,
    };
    let decoder = open_decoder(format, counted)?;
    let mut tar = tar::Archive::new(decoder);
    let mut files = 0u64;
    let mut seen_tops = std::collections::HashSet::new();
    for entry in tar.entries()? {
        cancel.check()?;
        let mut entry = entry?;
        let path = entry.path().map_err(Error::from)?;
        let rel = path.as_ref();
        if !is_safe_rel(rel) {
            return Err(Error::UnsafePath(rel.display().to_string()));
        }
        // A top-level folder this extraction creates is removed whole if it
        // fails.
        let mut parts = rel
            .components()
            .filter(|part| matches!(part, std::path::Component::Normal(_)));
        if let (Some(top), Some(_)) = (parts.next(), parts.next()) {
            let top = dest.join(top);
            if seen_tops.insert(top.clone()) && fs::symlink_metadata(&top).is_err() {
                created.push(top);
            }
        } else if entry.header().entry_type().is_dir() {
            let top = dest.join(rel);
            if seen_tops.insert(top.clone()) && fs::symlink_metadata(&top).is_err() {
                created.push(top);
            }
        }
        let fraction = Some(done.load(Ordering::Relaxed) as f64 / total as f64);
        report(&mut progress, fraction, rel.display().to_string());
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            let dir = checked_join(dest, rel)?;
            let mode = entry.header().mode().unwrap_or(0o755);
            prepare_dirs(dest, rel.parent().unwrap_or(Path::new("")))?;
            match fs::symlink_metadata(&dir) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                Ok(_) => return Err(Error::UnsafePath(dir.display().to_string())),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&dir)?;
                    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(mode & 0o777));
                }
                Err(err) => return Err(err.into()),
            }
            if let Ok(mtime) = entry.header().mtime() {
                transplant::set_mtime(&dir, mtime);
            }
            continue;
        }
        if kind == EntryType::Symlink {
            let Some(target) = entry.link_name().map_err(Error::from)? else {
                continue;
            };
            let dest_path = place_new(&checked_join(dest, rel)?, conflict)?;
            let Some(dest_path) = dest_path else { continue };
            prepare_dirs(dest, rel.parent().unwrap_or(Path::new("")))?;
            let _ = fs::remove_file(&dest_path);
            std::os::unix::fs::symlink(&target, &dest_path)?;
            created.push(dest_path);
            files += 1;
            continue;
        }
        if kind.is_file() || kind == EntryType::Regular || kind == EntryType::Continuous {
            let dest_path = place_new(&checked_join(dest, rel)?, conflict)?;
            let Some(dest_path) = dest_path else { continue };
            prepare_dirs(dest, rel.parent().unwrap_or(Path::new("")))?;
            let mode = entry.header().mode().unwrap_or(0o644);
            created.push(dest_path.clone());
            // `entry` is the reader. Copy it without following a planted symlink.
            let mut reader = CancelRead {
                inner: &mut entry,
                cancel,
            };
            transplant::write_new(&mut reader, &dest_path, mode, cancel)?;
            if let Ok(mtime) = entry.header().mtime() {
                transplant::set_mtime(&dest_path, mtime);
            }
            files += 1;
            continue;
        }
        if kind == EntryType::Link {
            // Hard link to another member. The target is a path inside the archive.
            if let Ok(Some(target)) = entry.link_name() {
                if is_safe_rel(target.as_ref()) {
                    let src = dest.join(target.as_ref());
                    if let Some(dest_path) = place_new(&checked_join(dest, rel)?, conflict)? {
                        prepare_dirs(dest, rel.parent().unwrap_or(Path::new("")))?;
                        if fs::hard_link(&src, &dest_path).is_ok() {
                            created.push(dest_path);
                        }
                        files += 1;
                    }
                }
            }
        }
    }
    report(&mut progress, Some(1.0), "Done");
    Ok(files)
}

pub fn extract_stream(
    archive: &Path,
    format: Format,
    dest_file: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let dest_file = match conflict {
        Conflict::Skip if dest_file.exists() => return Ok(0),
        Conflict::Rename => unique_path(dest_file),
        Conflict::Replace | Conflict::Skip => dest_file.to_path_buf(),
    };
    if let Some(parent) = dest_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::open(archive)?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(1).max(1);
    let done = Arc::new(AtomicU64::new(0));
    let counted = CountRead {
        inner: file,
        done: done.clone(),
        cancel: cancel.clone(),
        total,
    };
    let mut decoder = open_decoder(format, counted)?;
    let name = dest_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    report(&mut progress, Some(0.0), &name);
    transplant::write_new(&mut decoder, &dest_file, 0o644, cancel)?;
    report(
        &mut progress,
        Some(done.load(Ordering::Relaxed) as f64 / total as f64),
        &name,
    );
    Ok(1)
}

fn place_new(path: &Path, conflict: Conflict) -> Result<Option<PathBuf>> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(Some(path.to_path_buf()));
    }
    match conflict {
        Conflict::Skip => Ok(None),
        Conflict::Replace => {
            if let Ok(meta) = fs::symlink_metadata(path) {
                if meta.is_dir() && !meta.file_type().is_symlink() {
                    return Err(Error::Failed(format!(
                        "{} is a folder, so it was left in place.",
                        path.display()
                    )));
                }
            }
            let _ = fs::remove_file(path);
            Ok(Some(path.to_path_buf()))
        }
        Conflict::Rename => Ok(Some(unique_path(path))),
    }
}

// The arguments are the archive request's fields, passed through as is.
#[allow(clippy::too_many_arguments)]
fn append_path<W: Write>(
    builder: &mut Builder<W>,
    disk: &Path,
    archive_name: &str,
    exclude_macos: bool,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
    done: &mut u64,
    total: u64,
) -> Result<u64> {
    cancel.check()?;
    if exclude_macos && crate::names::is_macos_metadata(archive_name) {
        return Ok(0);
    }
    let meta = fs::symlink_metadata(disk)?;
    let mode = meta.permissions().mode();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if meta.file_type().is_symlink() {
        let target = fs::read_link(disk)?;
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(mode);
        header.set_mtime(mtime);
        header.set_cksum();
        builder
            .append_link(&mut header, archive_name, target)
            .map_err(Error::from)?;
        return Ok(0);
    }
    if meta.is_dir() {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Directory);
        header.set_size(0);
        header.set_mode(mode);
        header.set_mtime(mtime);
        header.set_cksum();
        let dirname = if archive_name.ends_with('/') {
            archive_name.to_string()
        } else {
            format!("{archive_name}/")
        };
        builder
            .append_data(&mut header, &dirname, io::empty())
            .map_err(Error::from)?;
        let mut count = 0u64;
        let mut children: Vec<_> = fs::read_dir(disk)?
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .map(|entry| entry.file_name())
            .collect();
        children.sort();
        for name in children {
            let child = disk.join(&name);
            let child_archive = format!("{archive_name}/{}", name.to_string_lossy());
            if exclude_macos && crate::names::is_macos_metadata(&child_archive) {
                continue;
            }
            count += append_path(
                builder,
                &child,
                &child_archive,
                exclude_macos,
                cancel,
                progress,
                done,
                total,
            )?;
        }
        return Ok(count);
    }
    if meta.is_file() {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        header.set_size(meta.len());
        header.set_mode(mode);
        header.set_mtime(mtime);
        header.set_cksum();
        let file = File::open(disk)?;
        let mut reader = ProgressRead {
            inner: file,
            cancel,
            done: *done,
            total,
            step: 0,
            label: archive_name.to_string(),
            progress,
        };
        builder
            .append_data(&mut header, archive_name, &mut reader)
            .map_err(Error::from)?;
        *done = reader.done;
        return Ok(1);
    }
    Ok(0)
}

fn measure(path: &Path, total: &mut u64) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        *total += meta.len();
    } else if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            measure(&entry?.path(), total)?;
        }
    }
    Ok(())
}

fn partial_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".into());
    output.with_file_name(format!(".{name}.partial"))
}

struct DeleteOnDrop<'a>(&'a Path);
impl Drop for DeleteOnDrop<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

enum Out {
    Plain(File),
    Gz(GzEncoder<File>),
    Xz(XzEncoder<File>),
    Bz(BzEncoder<File>),
    Zst(zstd::stream::write::Encoder<'static, File>),
}

impl Out {
    fn create(format: Format, level: Level, file: File) -> Result<Self> {
        Ok(match format {
            Format::Tar => Out::Plain(file),
            Format::TarGz | Format::Gz => Out::Gz(GzEncoder::new(file, GzLevel::new(level.gzip()))),
            Format::TarXz | Format::Xz => Out::Xz(XzEncoder::new(file, level.xz())),
            Format::TarBz2 | Format::Bz2 => {
                Out::Bz(BzEncoder::new(file, BzLevel::new(level.bzip2())))
            }
            Format::TarZst | Format::Zst => {
                Out::Zst(zstd::stream::write::Encoder::new(file, level.zstd())?)
            }
            _ => {
                return Err(Error::Failed(
                    "This format is not packed in process.".into(),
                ))
            }
        })
    }

    fn finish(self) -> Result<()> {
        let mut file = match self {
            Out::Plain(file) => file,
            Out::Gz(encoder) => encoder.finish()?,
            Out::Xz(encoder) => encoder.finish()?,
            Out::Bz(encoder) => encoder.finish()?,
            Out::Zst(encoder) => encoder.finish()?,
        };
        file.flush()?;
        let _ = file.sync_all();
        Ok(())
    }
}

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Out::Plain(file) => file.write(buf),
            Out::Gz(encoder) => encoder.write(buf),
            Out::Xz(encoder) => encoder.write(buf),
            Out::Bz(encoder) => encoder.write(buf),
            Out::Zst(encoder) => encoder.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Out::Plain(file) => file.flush(),
            Out::Gz(encoder) => encoder.flush(),
            Out::Xz(encoder) => encoder.flush(),
            Out::Bz(encoder) => encoder.flush(),
            Out::Zst(encoder) => encoder.flush(),
        }
    }
}

fn open_decoder(format: Format, reader: impl Read + 'static) -> Result<Box<dyn Read>> {
    Ok(match format {
        Format::Tar => Box::new(reader),
        Format::TarGz | Format::Gz => Box::new(GzDecoder::new(reader)),
        Format::TarXz | Format::Xz => Box::new(XzDecoder::new(reader)),
        Format::TarBz2 | Format::Bz2 => Box::new(BzDecoder::new(reader)),
        Format::TarZst | Format::Zst => Box::new(zstd::stream::read::Decoder::new(reader)?),
        _ => {
            return Err(Error::Failed(
                "This format is not a tar archive or a single compressed file.".into(),
            ))
        }
    })
}

struct CountRead<R> {
    inner: R,
    done: Arc<AtomicU64>,
    cancel: Cancel,
    #[allow(dead_code)]
    total: u64,
}

impl<R: Read> Read for CountRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(cancelled_io());
        }
        let n = self.inner.read(buf)?;
        self.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

struct CancelRead<'a, R> {
    inner: &'a mut R,
    cancel: &'a Cancel,
}

impl<R: Read> Read for CancelRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(cancelled_io());
        }
        self.inner.read(buf)
    }
}

struct ProgressRead<'a, R> {
    inner: R,
    cancel: &'a Cancel,
    done: u64,
    total: u64,
    step: u64,
    label: String,
    progress: &'a mut dyn FnMut(Progress),
}

impl<R: Read> Read for ProgressRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(cancelled_io());
        }
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        self.step += n as u64;
        if self.step >= 256 * 1024 || n == 0 {
            self.step = 0;
            let fraction = (self.done as f64 / self.total as f64).min(1.0);
            (self.progress)(Progress {
                fraction: Some(fraction),
                message: self.label.clone(),
            });
        }
        Ok(n)
    }
}
