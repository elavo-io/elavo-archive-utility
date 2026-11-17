//! Single-file compressors that are not built into the tar path: lzip, lz4,
//! brotli, lrzip, snappy, compress (.Z) and raw lzma. A folder is tarred
//! first and then passed through the same compressor.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use snap::read::FrameDecoder;
use snap::write::FrameEncoder;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::format::{Format, Level};
use crate::process::{self, Spec};
use crate::progress::{report, Progress};
use crate::settings::Conflict;
use crate::tarpack;
use crate::tools::{self, Tools};
use crate::transplant::unique_path;

pub fn compress(
    inputs: &[PathBuf],
    output: &Path,
    format: Format,
    level: Level,
    exclude_macos: bool,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let codec = format.tar_codec().unwrap_or(format);
    if format.is_tar_family() {
        let dir = tempfile::tempdir().map_err(Error::from)?;
        let tar_path = dir.path().join("payload.tar");
        let files = tarpack::compress_tree(
            inputs,
            &tar_path,
            Format::Tar,
            Level::Store,
            exclude_macos,
            cancel,
            &mut progress,
        )?;
        compress_one(&tar_path, output, codec, level, cancel, &mut progress)?;
        let _ = fs::remove_file(&tar_path);
        Ok(files)
    } else {
        compress_one(&inputs[0], output, codec, level, cancel, &mut progress)?;
        Ok(1)
    }
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
    let codec = format.tar_codec().unwrap_or(format);
    decode_to_file(archive, codec, &dest_file, cancel, &mut progress)?;
    Ok(1)
}

/// Decompress a tar wrapper into a plain tar, then unpack that.
pub fn extract_tar(
    archive: &Path,
    format: Format,
    dest: &Path,
    conflict: Conflict,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<u64> {
    let codec = format.tar_codec().ok_or_else(|| {
        Error::Failed("Internal error: this archive is not a compressed tar.".into())
    })?;
    let dir = tempfile::tempdir().map_err(Error::from)?;
    let tar_path = dir.path().join("payload.tar");
    decode_to_file(archive, codec, &tar_path, cancel, &mut progress)?;
    tarpack::extract_tar(&tar_path, Format::Tar, dest, conflict, cancel, progress)
}

pub fn test_file(archive: &Path, format: Format, cancel: &Cancel) -> Result<()> {
    let codec = format.tar_codec().unwrap_or(format);
    let dir = tempfile::tempdir().map_err(Error::from)?;
    let out = dir.path().join("check");
    decode_to_file(archive, codec, &out, cancel, &mut |_| {})?;
    if fs::metadata(&out).map(|meta| meta.len()).unwrap_or(0) == 0
        && fs::metadata(archive)?.len() > 32
    {
        return Err(Error::Failed(
            "Verification could not read the archive.".into(),
        ));
    }
    Ok(())
}

fn compress_one(
    input: &Path,
    output: &Path,
    codec: Format,
    level: Level,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    cancel.check()?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    if codec == Format::Snappy {
        return snappy_compress(input, output, cancel, progress);
    }
    let tools = tools::tools();
    let (program, args, stdout) = compress_command(tools, codec, level, input, output)?;
    let label = output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| codec.label().into());
    report(progress, None, &label);
    if stdout {
        let ran = process::run(
            Spec {
                program,
                args,
                current_dir: output.parent().map(Path::to_path_buf),
                writable: output.parent().map(Path::to_path_buf),
                stdin: None,
                stdout_path: Some(output.to_path_buf()),
            },
            cancel,
            |_| {},
        )?;
        if ran.code != Some(0) {
            let _ = fs::remove_file(output);
            return Err(Error::Failed(process::summarize(&ran.text)));
        }
    } else {
        let ran = process::run(
            Spec {
                program,
                args,
                current_dir: output.parent().map(Path::to_path_buf),
                writable: output.parent().map(Path::to_path_buf),
                stdin: None,
                stdout_path: None,
            },
            cancel,
            |_| {},
        )?;
        if ran.code != Some(0) {
            let _ = fs::remove_file(output);
            return Err(Error::Failed(process::summarize(&ran.text)));
        }
    }
    if !output.is_file() {
        return Err(Error::Failed("The compressor did not write a file.".into()));
    }
    report(progress, Some(1.0), label);
    Ok(())
}

/// `(program, args, write stdout to the destination)`.
fn compress_command(
    tools: &Tools,
    codec: Format,
    level: Level,
    input: &Path,
    output: &Path,
) -> Result<(PathBuf, Vec<String>, bool)> {
    let input = input.display().to_string();
    let output = output.display().to_string();
    match codec {
        Format::Lzip => {
            let program = tools
                .lzip
                .clone()
                .ok_or(missing("lzip", "create LZIP archives"))?;
            Ok((
                program,
                vec!["-c".into(), level.lzip().into(), "--".into(), input],
                true,
            ))
        }
        Format::Lz4 => {
            let program = tools
                .lz4
                .clone()
                .ok_or(missing("lz4", "create LZ4 archives"))?;
            Ok((
                program,
                vec![
                    "-z".into(),
                    "-q".into(),
                    "-f".into(),
                    "-c".into(),
                    level.lz4().into(),
                    "--".into(),
                    input,
                ],
                true,
            ))
        }
        Format::Brotli => {
            let program = tools
                .brotli
                .clone()
                .ok_or(missing("brotli", "create Brotli archives"))?;
            Ok((
                program,
                vec![
                    "-c".into(),
                    "-q".into(),
                    level.brotli().into(),
                    "--".into(),
                    input,
                ],
                true,
            ))
        }
        Format::Lrzip => {
            let program = tools
                .lrzip
                .clone()
                .ok_or(missing("lrzip", "create LRZIP archives"))?;
            Ok((
                program,
                vec![
                    "-q".into(),
                    "-f".into(),
                    "-L".into(),
                    level.lrzip_level().into(),
                    "-o".into(),
                    output,
                    "--".into(),
                    input,
                ],
                false,
            ))
        }
        _ => Err(Error::Failed(format!(
            "{} cannot be created.",
            codec.label()
        ))),
    }
}

fn decode_to_file(
    archive: &Path,
    codec: Format,
    dest: &Path,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    cancel.check()?;
    if codec == Format::Snappy {
        return snappy_decompress(archive, dest, cancel, progress);
    }
    let tools = tools::tools();
    let label = archive
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| codec.label().into());
    report(progress, None, &label);
    if let Some((program, args, stdout)) = decode_command(tools, codec, archive, dest)? {
        let ran = if stdout {
            process::run(
                Spec {
                    program,
                    args,
                    current_dir: dest.parent().map(Path::to_path_buf),
                    writable: dest.parent().map(Path::to_path_buf),
                    stdin: None,
                    stdout_path: Some(dest.to_path_buf()),
                },
                cancel,
                |_| {},
            )?
        } else {
            process::run(
                Spec {
                    program,
                    args,
                    current_dir: dest.parent().map(Path::to_path_buf),
                    writable: dest.parent().map(Path::to_path_buf),
                    stdin: None,
                    stdout_path: None,
                },
                cancel,
                |_| {},
            )?
        };
        if ran.code != Some(0) || !dest.is_file() {
            let _ = fs::remove_file(dest);
            return Err(tool_failure(codec, &ran.text));
        }
        report(progress, Some(1.0), "Done");
        return Ok(());
    }
    // Last resort for formats 7-Zip knows (.Z, lzma) when the dedicated
    // program is missing.
    if matches!(codec, Format::Compress | Format::Lzma) && tools.seven_zip.is_some() {
        return decode_with_seven(archive, dest, cancel, progress);
    }
    Err(missing(codec.extract_tool(), "open this"))
}

fn decode_command(
    tools: &Tools,
    codec: Format,
    archive: &Path,
    dest: &Path,
) -> Result<Option<(PathBuf, Vec<String>, bool)>> {
    let archive = archive.display().to_string();
    let dest = dest.display().to_string();
    let found = match codec {
        Format::Lzip => tools.lzip.clone().map(|program| {
            (
                program,
                vec!["-d".into(), "-c".into(), "-f".into(), "--".into(), archive],
                true,
            )
        }),
        Format::Lz4 => tools.lz4.clone().map(|program| {
            (
                program,
                vec![
                    "-d".into(),
                    "-q".into(),
                    "-f".into(),
                    "-c".into(),
                    "--".into(),
                    archive,
                ],
                true,
            )
        }),
        Format::Brotli => tools.brotli.clone().map(|program| {
            (
                program,
                vec!["-d".into(), "-c".into(), "-f".into(), "--".into(), archive],
                true,
            )
        }),
        Format::Lrzip => tools.lrzip.clone().map(|program| {
            (
                program,
                vec![
                    "-d".into(),
                    "-q".into(),
                    "-f".into(),
                    "-o".into(),
                    dest,
                    "--".into(),
                    archive,
                ],
                false,
            )
        }),
        Format::Compress => tools
            .gzip
            .clone()
            .map(|program| {
                (
                    program,
                    vec![
                        "-d".into(),
                        "-c".into(),
                        "-f".into(),
                        "--".into(),
                        archive.clone(),
                    ],
                    true,
                )
            })
            .or_else(|| {
                let archive = archive.clone();
                tools
                    .uncompress
                    .clone()
                    .map(move |program| (program, vec!["-c".into(), archive], true))
            }),
        Format::Lzma => tools
            .xz
            .clone()
            .map(|program| {
                (
                    program,
                    vec![
                        "-d".into(),
                        "-c".into(),
                        "--format=lzma".into(),
                        "-f".into(),
                        "--".into(),
                        archive.clone(),
                    ],
                    true,
                )
            })
            .or_else(|| {
                let archive = archive.clone();
                tools.lzma.clone().map(move |program| {
                    (
                        program,
                        vec!["-d".into(), "-c".into(), "-f".into(), "--".into(), archive],
                        true,
                    )
                })
            }),
        _ => None,
    };
    if found.is_none() && !matches!(codec, Format::Compress | Format::Lzma) {
        return Err(missing(codec.extract_tool(), "open this"));
    }
    Ok(found)
}

fn decode_with_seven(
    archive: &Path,
    dest: &Path,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let program = tools::tools()
        .seven_zip
        .clone()
        .ok_or(missing("7z", "open this"))?;
    let dir = tempfile::tempdir().map_err(Error::from)?;
    let ran = process::run(
        Spec {
            program,
            args: vec![
                "x".into(),
                "-y".into(),
                format!("-o{}", dir.path().display()),
                "--".into(),
                archive.display().to_string(),
            ],
            current_dir: Some(dir.path().to_path_buf()),
            writable: Some(dir.path().to_path_buf()),
            stdin: None,
            stdout_path: None,
        },
        cancel,
        |_| {},
    )?;
    if ran.code != Some(0) && ran.code != Some(1) {
        return Err(Error::Failed(process::summarize(&ran.text)));
    }
    let produced = first_file(dir.path())?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&produced, dest).or_else(|_| {
        fs::copy(&produced, dest)?;
        let _ = fs::remove_file(&produced);
        Ok::<(), Error>(())
    })?;
    report(progress, Some(1.0), "Done");
    Ok(())
}

fn first_file(dir: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            return Ok(entry.path());
        }
        if entry.file_type()?.is_dir() {
            if let Ok(path) = first_file(&entry.path()) {
                return Ok(path);
            }
        }
    }
    Err(Error::Failed(
        "The archive tool did not write a file.".into(),
    ))
}

fn snappy_compress(
    input: &Path,
    output: &Path,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let infile = File::open(input)?;
    let outfile = File::create(output)?;
    let mut encoder = FrameEncoder::new(outfile);
    copy_cancel(infile, &mut encoder, cancel)?;
    let mut file = encoder
        .into_inner()
        .map_err(|err| Error::Failed(err.to_string()))?;
    file.flush()?;
    report(progress, Some(1.0), "Done");
    Ok(())
}

fn snappy_decompress(
    archive: &Path,
    dest: &Path,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let infile = File::open(archive)?;
    let outfile = File::create(dest)?;
    let mut decoder = FrameDecoder::new(infile);
    copy_cancel(&mut decoder, outfile, cancel).map_err(|err| {
        if err.kind() == io::ErrorKind::InvalidData {
            Error::Failed("This is not a Snappy frame.".into())
        } else {
            Error::from(err)
        }
    })?;
    report(progress, Some(1.0), "Done");
    Ok(())
}

fn copy_cancel(mut reader: impl Read, mut writer: impl Write, cancel: &Cancel) -> io::Result<u64> {
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
    writer.flush()?;
    Ok(total)
}

fn missing(tool: &'static str, purpose: &'static str) -> Error {
    Error::ToolMissing {
        tool,
        purpose: purpose.into(),
    }
}

fn tool_failure(codec: Format, text: &str) -> Error {
    let summary = process::summarize(text);
    if summary == "The archive tool failed." {
        missing(codec.extract_tool(), "open this")
    } else {
        Error::Failed(summary)
    }
}
