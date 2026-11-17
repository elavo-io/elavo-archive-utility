use std::fs;
use std::path::{Path, PathBuf};

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::foreign;
use crate::format::{self, Format, Level};
use crate::names::{self, archive_stem, suggest_archive_path};
use crate::progress::Progress;
use crate::settings::{Conflict, NewFolder, OutputPlace};
use crate::{rar, seven, streams, tarpack};

#[derive(Clone, Debug)]
pub struct CompressRequest {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub format: Format,
    pub level: Level,
    pub password: Option<String>,
    pub encrypt_names: bool,
    /// ZIP only. 7z already uses AES when a password is set.
    pub aes256: bool,
    /// Split the archive into pieces of this many bytes. 7z and ZIP only.
    pub split: Option<u64>,
    pub exclude_macos: bool,
    pub verify: bool,
    pub delete_after: bool,
    pub separate: bool,
}

impl Default for CompressRequest {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output: PathBuf::new(),
            format: Format::SevenZ,
            level: Level::Normal,
            password: None,
            encrypt_names: false,
            aes256: false,
            split: None,
            exclude_macos: true,
            verify: false,
            delete_after: false,
            separate: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtractRequest {
    pub archive: PathBuf,
    pub destination: PathBuf,
    pub password: Option<String>,
    pub new_folder: NewFolder,
    pub conflict: Conflict,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub path: PathBuf,
    pub files: u64,
    pub warnings: Vec<String>,
}

pub fn compress(
    req: &CompressRequest,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<Outcome> {
    cancel.check()?;
    if req.inputs.is_empty() {
        return Err(Error::Empty);
    }
    for input in &req.inputs {
        if fs::symlink_metadata(input).is_err() {
            return Err(Error::Failed(format!("Can't find {}.", input.display())));
        }
    }
    if req.separate && req.inputs.len() > 1 {
        return compress_separate(req, cancel, progress);
    }
    let mut one = req.clone();
    one.format = req.format.for_inputs(&req.inputs);
    one.separate = false;
    let outcome = compress_one(&one, cancel, &mut progress)?;
    Ok(outcome)
}

fn compress_separate(
    req: &CompressRequest,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let parent = req
        .output
        .parent()
        .map(Path::to_path_buf)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut last = None;
    let mut files = 0u64;
    for input in &req.inputs {
        cancel.check()?;
        let format = req.format.for_inputs(std::slice::from_ref(input));
        let output = suggest_archive_path(
            std::slice::from_ref(input),
            format,
            &OutputPlace::Directory(parent.clone()),
        );
        let one = CompressRequest {
            inputs: vec![input.clone()],
            output,
            format,
            separate: false,
            ..req.clone()
        };
        let outcome = compress_one(&one, cancel, &mut progress)?;
        files += outcome.files;
        last = Some(outcome.path);
    }
    Ok(Outcome {
        path: last.unwrap_or(parent),
        files,
        warnings: Vec::new(),
    })
}

fn compress_one(
    req: &CompressRequest,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<Outcome> {
    if !req.format.can_compress() {
        return Err(Error::Unsupported {
            format: req.format.label().to_string(),
            action: "be created",
        });
    }
    if !req.format.create_ready(crate::tools::tools()) {
        return Err(Error::Failed(req.format.create_hint().into()));
    }
    if req.split.is_some() && !req.format.supports_split() {
        return Err(Error::Failed(format!(
            "{} archives are not split into volumes.",
            req.format.label()
        )));
    }
    let password = req.password.as_deref().filter(|p| !p.is_empty());
    if password.is_some() && !req.format.supports_password() {
        return Err(Error::Failed(format!(
            "{} does not take a password.",
            req.format.label()
        )));
    }

    let files = dispatch_compress(req, password, cancel, progress)?;
    if req.verify {
        verify(req, password, cancel)?;
    }
    if req.delete_after {
        delete_inputs(&req.inputs, &req.output)?;
    }
    let path = first_existing(&req.output);
    Ok(Outcome {
        path,
        files,
        warnings: Vec::new(),
    })
}

fn dispatch_compress(
    req: &CompressRequest,
    password: Option<&str>,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    match req.format {
        Format::SevenZ | Format::Zip | Format::Wim => seven::compress(
            &req.inputs,
            &req.output,
            req.format,
            req.level,
            password,
            req.encrypt_names,
            req.aes256,
            req.split,
            req.exclude_macos,
            cancel,
            progress,
        ),
        Format::Iso => foreign::create_iso(
            &req.inputs,
            &req.output,
            req.exclude_macos,
            cancel,
            progress,
        ),
        Format::Dmg => foreign::create_dmg(
            &req.inputs,
            &req.output,
            req.exclude_macos,
            cancel,
            progress,
        ),
        Format::Tar | Format::TarGz | Format::TarXz | Format::TarBz2 | Format::TarZst => {
            tarpack::compress_tree(
                &req.inputs,
                &req.output,
                req.format,
                req.level,
                req.exclude_macos,
                cancel,
                progress,
            )
        }
        Format::Gz | Format::Xz | Format::Bz2 | Format::Zst => tarpack::compress_stream(
            &req.inputs[0],
            &req.output,
            req.format,
            req.level,
            cancel,
            progress,
        ),
        Format::Lzip
        | Format::Lz4
        | Format::Brotli
        | Format::Lrzip
        | Format::Snappy
        | Format::TarLz
        | Format::TarLz4
        | Format::TarBr
        | Format::TarLrz
        | Format::TarSz => streams::compress(
            &req.inputs,
            &req.output,
            req.format,
            req.level,
            req.exclude_macos,
            cancel,
            progress,
        ),
        other => Err(Error::Unsupported {
            format: other.label().to_string(),
            action: "be created",
        }),
    }
}

fn verify(req: &CompressRequest, password: Option<&str>, cancel: &Cancel) -> Result<()> {
    match req.format {
        Format::SevenZ | Format::Zip | Format::Wim => {
            seven::test_archive(&first_existing(&req.output), password, cancel)
        }
        Format::Tar | Format::TarGz | Format::TarXz | Format::TarBz2 | Format::TarZst => {
            let names = tarpack::list_tar(&req.output, req.format, cancel)?;
            if names.is_empty() {
                return Err(Error::Failed(
                    "Verification could not read the archive.".into(),
                ));
            }
            Ok(())
        }
        Format::Gz | Format::Xz | Format::Bz2 | Format::Zst => {
            let dir = tempfile::tempdir().map_err(Error::from)?;
            let dest = dir.path().join("check");
            tarpack::extract_stream(
                &req.output,
                req.format,
                &dest,
                Conflict::Replace,
                cancel,
                |_| {},
            )?;
            Ok(())
        }
        Format::Lzip
        | Format::Lz4
        | Format::Brotli
        | Format::Lrzip
        | Format::Snappy
        | Format::TarLz
        | Format::TarLz4
        | Format::TarBr
        | Format::TarLrz
        | Format::TarSz => streams::test_file(&req.output, req.format, cancel),
        Format::Iso | Format::Dmg => Ok(()),
        _ => Ok(()),
    }
}

fn delete_inputs(inputs: &[PathBuf], output: &Path) -> Result<()> {
    for input in inputs {
        if !safe_to_delete(input, output) {
            continue;
        }
        let meta = fs::symlink_metadata(input)?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(input)?;
        } else {
            fs::remove_file(input)?;
        }
    }
    Ok(())
}

fn safe_to_delete(input: &Path, output: &Path) -> bool {
    if input == Path::new("/") || input.parent().is_none() {
        return false;
    }
    if let Some(home) = std::env::var_os("HOME") {
        if input == Path::new(&home) {
            return false;
        }
    }
    let input_canon = fs::canonicalize(input).unwrap_or_else(|_| input.to_path_buf());
    let output_canon = fs::canonicalize(output).unwrap_or_else(|_| output.to_path_buf());
    if output_canon.starts_with(&input_canon) {
        return false;
    }
    true
}

pub fn extract(
    req: &ExtractRequest,
    cancel: &Cancel,
    progress: impl FnMut(Progress),
) -> Result<Outcome> {
    cancel.check()?;
    if fs::symlink_metadata(&req.archive).is_err() {
        return Err(Error::Failed(format!(
            "Can't find {}.",
            req.archive.display()
        )));
    }
    let format = format::detect(&req.archive);
    // An unrecognised file is handed to 7-Zip before Archive Utility gives up.
    let format = match format {
        Some(format) => format,
        None => {
            return extract_unknown(req, cancel, progress);
        }
    };
    extract_known(req, format, cancel, progress)
}

fn extract_unknown(
    req: &ExtractRequest,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<Outcome> {
    if crate::tools::tools().seven_zip.is_none() && crate::tools::tools().bsdtar.is_none() {
        return Err(Error::ToolMissing {
            tool: "7z",
            purpose: "open this".into(),
        });
    }
    let stem = archive_stem(
        &req.archive
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".into()),
    );
    fs::create_dir_all(&req.destination)?;
    let folder = match req.new_folder {
        NewFolder::Never => req.destination.clone(),
        NewFolder::Always | NewFolder::WhenSeveral => req.destination.join(stem),
    };
    fs::create_dir_all(&folder)?;
    let password = req.password.as_deref().filter(|p| !p.is_empty());
    let files = seven::extract(
        &req.archive,
        &folder,
        Format::SevenZ,
        password,
        req.conflict,
        cancel,
        &mut progress,
    )?;
    Ok(Outcome {
        path: folder,
        files,
        warnings: Vec::new(),
    })
}

fn extract_known(
    req: &ExtractRequest,
    format: Format,
    cancel: &Cancel,
    mut progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let password = req.password.as_deref().filter(|p| !p.is_empty());
    let stem = req
        .archive
        .file_name()
        .map(|n| archive_stem(&n.to_string_lossy()))
        .unwrap_or_else(|| "archive".into());
    fs::create_dir_all(&req.destination)?;

    let folder = match req.new_folder {
        NewFolder::Never => req.destination.clone(),
        NewFolder::Always => req.destination.join(&stem),
        NewFolder::WhenSeveral => match member_names(&req.archive, format, password, cancel) {
            Ok(names) => {
                if names::toplevel_count(names) > 1 {
                    req.destination.join(&stem)
                } else {
                    req.destination.clone()
                }
            }
            Err(Error::PasswordRequired) => return Err(Error::PasswordRequired),
            Err(Error::WrongPassword) => return Err(Error::WrongPassword),
            Err(_) => req.destination.join(&stem),
        },
    };
    fs::create_dir_all(&folder)?;

    let files = dispatch_extract(req, format, &folder, password, cancel, &mut progress)?;
    Ok(Outcome {
        path: folder,
        files,
        warnings: Vec::new(),
    })
}

fn dispatch_extract(
    req: &ExtractRequest,
    format: Format,
    folder: &Path,
    password: Option<&str>,
    cancel: &Cancel,
    progress: &mut impl FnMut(Progress),
) -> Result<u64> {
    if format.is_stream()
        && !matches!(
            format,
            Format::Lzip
                | Format::Lz4
                | Format::Brotli
                | Format::Lrzip
                | Format::Snappy
                | Format::Compress
                | Format::Lzma
        )
    {
        let file_name = stream_name(&req.archive);
        return tarpack::extract_stream(
            &req.archive,
            format,
            &folder.join(file_name),
            req.conflict,
            cancel,
            progress,
        );
    }
    if matches!(
        format,
        Format::Lzip
            | Format::Lz4
            | Format::Brotli
            | Format::Lrzip
            | Format::Snappy
            | Format::Compress
            | Format::Lzma
    ) {
        let file_name = stream_name(&req.archive);
        return streams::extract_stream(
            &req.archive,
            format,
            &folder.join(file_name),
            req.conflict,
            cancel,
            progress,
        );
    }
    if matches!(
        format,
        Format::Tar | Format::TarGz | Format::TarXz | Format::TarBz2 | Format::TarZst
    ) {
        return tarpack::extract_tar(&req.archive, format, folder, req.conflict, cancel, progress);
    }
    if matches!(
        format,
        Format::TarLz
            | Format::TarLz4
            | Format::TarBr
            | Format::TarLrz
            | Format::TarSz
            | Format::TarZ
            | Format::TarLzma
    ) {
        return streams::extract_tar(&req.archive, format, folder, req.conflict, cancel, progress);
    }
    if format == Format::Rar {
        return rar::extract(
            &req.archive,
            folder,
            password,
            req.conflict,
            cancel,
            progress,
        );
    }
    if format == Format::Deb {
        if let Some(program) = crate::tools::tools().dpkg_deb.clone() {
            return extract_deb(&program, &req.archive, folder, req.conflict, cancel);
        }
    }
    if format == Format::Exe {
        return foreign::extract_exe(
            &req.archive,
            folder,
            password,
            req.conflict,
            cancel,
            progress,
        );
    }
    // ZIP, 7z, and everything 7-Zip (or bsdtar, or cabextract) can open.
    seven::extract(
        &req.archive,
        folder,
        format,
        password,
        req.conflict,
        cancel,
        progress,
    )
}

fn member_names(
    archive: &Path,
    format: Format,
    password: Option<&str>,
    cancel: &Cancel,
) -> Result<Vec<String>> {
    if format.is_stream() {
        return Ok(vec![stream_name(archive)]);
    }
    if matches!(
        format,
        Format::Tar | Format::TarGz | Format::TarXz | Format::TarBz2 | Format::TarZst
    ) {
        return tarpack::list_tar(archive, format, cancel);
    }
    if format == Format::Rar {
        let names = rar::list(archive, password, cancel)?;
        if names.is_empty() {
            return Ok(vec!["item".into(), "other".into()]);
        }
        return Ok(names);
    }
    if crate::tools::tools().seven_zip.is_some() {
        return seven::list(archive, password, cancel);
    }
    Err(Error::Failed("No listing tool is installed.".into()))
}

fn extract_deb(
    program: &Path,
    archive: &Path,
    folder: &Path,
    conflict: Conflict,
    cancel: &Cancel,
) -> Result<u64> {
    let work = tempfile::tempdir().map_err(Error::from)?;
    let ran = crate::process::run(
        crate::process::Spec {
            program: program.to_path_buf(),
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
        return Err(Error::Failed(crate::process::summarize(&ran.text)));
    }
    crate::transplant::transplant(work.path(), folder, conflict, cancel)
}

fn stream_name(archive: &Path) -> String {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    Path::new(&name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "file".into())
}

fn first_existing(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let numbered = output.with_file_name(format!("{name}.001"));
    if numbered.is_file() {
        return numbered;
    }
    // Info-ZIP split: notes.z01, notes.z02, notes.zip. Extraction starts
    // at the first part.
    if let Some(stem) = output.file_stem() {
        let z01 = output.with_file_name(format!("{}.z01", stem.to_string_lossy()));
        if z01.is_file() {
            return z01;
        }
    }
    output.to_path_buf()
}
