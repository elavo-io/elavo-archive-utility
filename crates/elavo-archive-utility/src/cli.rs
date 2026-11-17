//! Command line compress and extract, without opening a window.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use elavo_archive_utility_core::{
    compress, extract, parse_size, Cancel, CompressRequest, Conflict, ExtractRequest, Format,
    Level, NewFolder, Progress,
};

pub fn wants(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "-h" | "--help" | "--version" | "--batch"))
}

pub fn run(args: &[String]) -> i32 {
    let mut rest = args.iter().skip(1).map(String::as_str);
    match rest.next() {
        Some("-h" | "--help") => {
            print!("{HELP}");
            0
        }
        Some("--version") => {
            println!("elavo-archive-utility {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--batch") => match rest.next() {
            Some("compress") => batch_compress(&args[3..]),
            Some("extract") => batch_extract(&args[3..]),
            _ => {
                eprintln!("Use: elavo-archive-utility --batch compress …   or   elavo-archive-utility --batch extract …");
                2
            }
        },
        _ => {
            print!("{HELP}");
            2
        }
    }
}

const HELP: &str = "\
Archive Utility compresses files and extracts archives.

Usage:
  elavo-archive-utility [FILE...]                 Open the window
  elavo-archive-utility --compact                 Open the small drop zone
  elavo-archive-utility --batch compress [options] FILE...
  elavo-archive-utility --batch extract [options] ARCHIVE

Compress options:
  --format FORMAT     7z, zip, tar, zstd, gz, bz2, xz, lzip, lz4,
                      brotli, lrzip, snappy, wim, or iso. Default: 7z
                      A folder is stored in a tar archive before a
                      single-file compressor (tar.zst, tar.gz, …).
  --output PATH       Archive to write
  --level LEVEL       store, fastest, faster, normal, slower, or slowest.
                      fast and slow still select the Fast and Slow stops
  --password TEXT     Encrypt, for 7z and ZIP. Sent on the tool's
                      standard input, not as an argument.
  --encrypt-names     Encrypt 7z file names as well
  --aes256            Use AES-256 for a ZIP password
  --split SIZE        For example: 5 MB, 100M, 700MB, 4.7GB
  --exclude-macos     Skip .DS_Store, AppleDouble, and __MACOSX
  --verify            Read the archive back after writing it
  --delete-after      Delete the originals after a successful archive
  --separate          Write one archive per item

Extract options:
  --output DIR        Folder to extract into
  --password TEXT     Sent on standard input when the tool reads a prompt
  --new-folder MODE   always, several, or never
  --conflict MODE     rename, replace, or skip

The password is not saved, and it is not put on the command line when
the archive tool can read it from standard input or a prompt.
";

fn batch_compress(args: &[String]) -> i32 {
    let mut format = Format::SevenZ;
    let mut output: Option<PathBuf> = None;
    let mut level = Level::Normal;
    let mut password = None;
    let mut encrypt_names = false;
    let mut aes256 = false;
    let mut split = None;
    let mut exclude_macos = false;
    let mut verify = false;
    let mut delete_after = false;
    let mut separate = false;
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = |arg: &str| -> Result<&str, i32> {
            args.get(i + 1).map(String::as_str).ok_or_else(|| {
                eprintln!("Missing a value after {arg}");
                2
            })
        };
        match arg {
            "--format" => {
                let text = match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                };
                format = match Format::from_id(text) {
                    Some(format) if format.can_compress() => format,
                    _ => {
                        eprintln!("Unknown format {text}");
                        return 2;
                    }
                };
                i += 2;
            }
            "--output" | "-o" => {
                output = Some(PathBuf::from(match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                }));
                i += 2;
            }
            "--level" => {
                level = Level::from_id(match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                });
                i += 2;
            }
            "--password" => {
                password = Some(
                    match value(arg) {
                        Ok(v) => v,
                        Err(code) => return code,
                    }
                    .to_string(),
                );
                i += 2;
            }
            "--encrypt-names" => {
                encrypt_names = true;
                i += 1;
            }
            "--aes256" => {
                aes256 = true;
                i += 1;
            }
            "--exclude-macos" => {
                exclude_macos = true;
                i += 1;
            }
            "--verify" => {
                verify = true;
                i += 1;
            }
            "--delete-after" => {
                delete_after = true;
                i += 1;
            }
            "--separate" => {
                separate = true;
                i += 1;
            }
            "--split" => {
                split = match parse_size(match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                }) {
                    Some(bytes) => bytes,
                    None => {
                        eprintln!("Couldn't read the split size. Example: 5 MB");
                        return 2;
                    }
                };
                i += 2;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option {other}");
                return 2;
            }
            other => {
                files.push(PathBuf::from(other));
                i += 1;
            }
        }
    }
    let Some(output) = output else {
        eprintln!("Name the archive with --output");
        return 2;
    };
    if files.is_empty() {
        eprintln!("Name the files to compress");
        return 2;
    }
    let req = CompressRequest {
        inputs: files,
        output,
        format,
        level,
        password,
        encrypt_names,
        aes256,
        split,
        exclude_macos,
        verify,
        delete_after,
        separate,
    };
    match compress(&req, &Cancel::new(), report) {
        Ok(outcome) => {
            println!("{}", outcome.path.display());
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn batch_extract(args: &[String]) -> i32 {
    let mut output: Option<PathBuf> = None;
    let mut password = None;
    let mut new_folder = NewFolder::WhenSeveral;
    let mut conflict = Conflict::Rename;
    let mut archive = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = |arg: &str| -> Result<&str, i32> {
            args.get(i + 1).map(String::as_str).ok_or_else(|| {
                eprintln!("Missing a value after {arg}");
                2
            })
        };
        match arg {
            "--output" | "-o" => {
                output = Some(PathBuf::from(match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                }));
                i += 2;
            }
            "--password" => {
                password = Some(
                    match value(arg) {
                        Ok(v) => v,
                        Err(code) => return code,
                    }
                    .to_string(),
                );
                i += 2;
            }
            "--new-folder" => {
                new_folder = NewFolder::from_id(match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                });
                i += 2;
            }
            "--conflict" => {
                conflict = Conflict::from_id(match value(arg) {
                    Ok(v) => v,
                    Err(code) => return code,
                });
                i += 2;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option {other}");
                return 2;
            }
            other => {
                archive = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    let (Some(output), Some(archive)) = (output, archive) else {
        eprintln!("Use --output DIR and name the archive");
        return 2;
    };
    let req = ExtractRequest {
        archive,
        destination: output,
        password,
        new_folder,
        conflict,
    };
    match extract(&req, &Cancel::new(), report) {
        Ok(outcome) => {
            println!("{}", outcome.path.display());
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn report(progress: Progress) {
    if std::io::stderr().is_terminal() {
        if let Some(fraction) = progress.fraction {
            let _ = write!(
                std::io::stderr(),
                "\r{:>3}%  {}",
                (fraction * 100.0) as u32,
                progress.message
            );
            let _ = std::io::stderr().flush();
        }
    }
}
