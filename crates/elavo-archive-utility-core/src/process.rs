use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::tools::{self, args_contain_secret, redact};

pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub current_dir: Option<PathBuf>,
    /// When bubblewrap works, only this directory is writable.
    pub writable: Option<PathBuf>,
    /// Bytes written to the program's stdin, then the pipe is closed.
    /// Passwords go here, never in `args`.
    pub stdin: Option<Vec<u8>>,
    /// When set, stdout is this file instead of the captured text.
    pub stdout_path: Option<PathBuf>,
}

pub struct Output {
    pub code: Option<i32>,
    pub text: String,
}

/// Run a program, streaming combined output to `on_chunk`.
///
/// A password is sent on stdin (7-Zip reads it when `-p` has no value) and
/// is refused if it also appears in an argument. When bubblewrap is available
/// the process may only write inside `writable`.
pub fn run(spec: Spec, cancel: &Cancel, mut on_chunk: impl FnMut(&[u8])) -> Result<Output> {
    cancel.check()?;
    if let Some(stdin) = &spec.stdin {
        let secret = String::from_utf8_lossy(stdin);
        let secret = secret.trim();
        if args_contain_secret(&spec.args, secret) {
            return Err(Error::Failed(
                "Refusing to put the password on the command line.".into(),
            ));
        }
    }
    let tools = tools::tools();
    let jail = spec
        .writable
        .as_ref()
        .filter(|_| tools.bwrap_works)
        .and_then(|_| tools.bwrap.clone());

    let mut command = if let (Some(bwrap), Some(writable)) = (jail, spec.writable.as_ref()) {
        let mut command = Command::new(bwrap);
        command
            .arg("--unshare-all")
            .arg("--die-with-parent")
            .arg("--ro-bind")
            .arg("/")
            .arg("/")
            .arg("--bind")
            .arg(writable)
            .arg(writable)
            .arg("--dev")
            .arg("/dev")
            .arg("--proc")
            .arg("/proc")
            .arg("--setenv")
            .arg("TMPDIR")
            .arg(writable)
            .arg("--setenv")
            .arg("HOME")
            .arg(writable)
            .arg("--setenv")
            .arg("LC_ALL")
            .arg("C")
            .arg("--setenv")
            .arg("LANG")
            .arg("C");
        if let Some(dir) = &spec.current_dir {
            command.arg("--chdir").arg(dir);
        }
        command.arg("--").arg(&spec.program);
        command.args(&spec.args);
        command
    } else {
        let mut command = Command::new(&spec.program);
        command.args(&spec.args);
        if let Some(dir) = &spec.current_dir {
            command.current_dir(dir);
        }
        command.env("LC_ALL", "C").env("LANG", "C");
        if let Some(dir) = &spec.writable {
            command.env("TMPDIR", dir);
        }
        command
    };

    if std::env::var_os("ELAVO_ARCHIVE_UTILITY_LOG").is_some() {
        eprintln!(
            "elavo-archive-utility: {} {}",
            spec.program.display(),
            redact(&spec.args).join(" ")
        );
    }

    if let Some(path) = &spec.stdout_path {
        let file = File::create(path)?;
        command.stdout(Stdio::from(file));
    } else {
        command.stdout(Stdio::piped());
    }
    command
        .process_group(0)
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|err| Error::Failed(format!("Couldn't run {}: {err}", spec.program.display())))?;
    cancel.register(child.id());
    if let Some(bytes) = spec.stdin.clone() {
        if let Some(mut input) = child.stdin.take() {
            std::thread::spawn(move || {
                let _ = input.write_all(&bytes);
            });
        }
    }
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let flag = Arc::new(AtomicBool::new(false));
    let stderr_text = {
        let flag = flag.clone();
        std::thread::spawn(move || {
            let mut collected = Vec::new();
            if let Some(err) = stderr.as_mut() {
                let mut buf = [0u8; 8192];
                while !flag.load(Ordering::Relaxed) {
                    match err.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => collected.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
            }
            collected
        })
    };

    let mut combined = Vec::new();
    if let Some(out) = stdout.as_mut() {
        let mut buf = [0u8; 8192];
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    on_chunk(&buf[..n]);
                    push_tail(&mut combined, &buf[..n]);
                }
                Err(_) => break,
            }
        }
    }

    if cancel.is_cancelled() {
        cancel.cancel();
    }
    // Don't wait forever if the child ignores the signal.
    let code = wait_child(&mut child, Duration::from_secs(2));
    flag.store(true, Ordering::Relaxed);
    if let Ok(err_bytes) = stderr_text.join() {
        push_tail(&mut combined, &err_bytes);
    }
    cancel.clear_pid();

    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let text = String::from_utf8_lossy(&combined).into_owned();
    Ok(Output { code, text })
}

fn wait_child(child: &mut std::process::Child, grace: Duration) -> Option<i32> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) if start.elapsed() > grace => {
                let _ = child.kill();
                return child.wait().ok().and_then(|status| status.code());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
}

fn push_tail(buf: &mut Vec<u8>, chunk: &[u8]) {
    buf.extend_from_slice(chunk);
    const MAX: usize = 256 * 1024;
    if buf.len() > MAX {
        let cut = buf.len() - MAX / 2;
        buf.drain(..cut);
    }
}

pub fn last_percent(text: &str) -> Option<u8> {
    let bytes = text.as_bytes();
    let mut last = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'%' {
                if let Ok(n) = text[start..i].parse::<u8>() {
                    if n <= 100 {
                        last = Some(n);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    last
}

/// Apply backspaces the way 7-Zip redraws its progress line.
pub fn apply_controls(buf: &mut Vec<u8>, chunk: &[u8]) {
    for &byte in chunk {
        match byte {
            0x08 | b'\r' => {
                buf.pop();
            }
            b'\n' => buf.clear(),
            _ => buf.push(byte),
        }
    }
    if buf.len() > 4096 {
        let cut = buf.len() - 1024;
        buf.drain(..cut);
    }
}

pub fn password_problem(text: &str, supplied: bool) -> Option<Error> {
    let lower = text.to_lowercase();
    let hit = lower.contains("wrong password")
        || lower.contains("enter password")
        || lower.contains("cannot open encrypted")
        || lower.contains("bad password")
        || lower.contains("incorrect password");
    if !hit {
        return None;
    }
    if supplied
        && (lower.contains("wrong")
            || lower.contains("bad password")
            || lower.contains("incorrect")
            || lower.contains("cannot open encrypted"))
    {
        Some(Error::WrongPassword)
    } else if supplied {
        Some(Error::WrongPassword)
    } else {
        Some(Error::PasswordRequired)
    }
}

pub fn summarize(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            if line.is_empty() {
                return false;
            }
            let lower = line.to_lowercase();
            !(lower.contains("copyright")
                || lower.starts_with("7-zip")
                || lower.starts_with("64-bit")
                || lower.starts_with("scanning the drive")
                || lower.starts_with("open_max")
                || lower.starts_with("files:")
                || lower.starts_with("size:")
                || lower.starts_with("compressed:"))
        })
        .collect();
    let tail: Vec<&str> = lines.into_iter().rev().take(6).collect();
    let mut ordered: Vec<&str> = tail.into_iter().rev().collect();
    if ordered.len() > 4 {
        ordered = ordered.split_off(ordered.len() - 4);
    }
    let message = ordered.join(" ");
    if message.is_empty() {
        "The archive tool failed.".to_string()
    } else {
        message
    }
}

/// Run a program with the password typed on its controlling terminal.
///
/// `unar` reads a password with `getpass()`, which uses `/dev/tty` rather
/// than stdin, so a pipe would not reach it. The password is written to a
/// pseudoterminal and is not placed in the argument list.
pub fn run_on_pty(spec: Spec, password: &str, cancel: &Cancel) -> Result<Output> {
    run_on_pty_times(spec, password, 1, cancel)
}

/// Like [`run_on_pty`], writing the password `times` times. Info-ZIP asks twice.
pub fn run_on_pty_times(
    spec: Spec,
    password: &str,
    times: u8,
    cancel: &Cancel,
) -> Result<Output> {
    cancel.check()?;
    if args_contain_secret(&spec.args, password) {
        return Err(Error::Failed(
            "Refusing to put the password on the command line.".into(),
        ));
    }
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if opened != 0 {
        return Err(Error::Failed(
            "Couldn't open a terminal for the password prompt.".into(),
        ));
    }
    let slave_fd = slave;
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(dir) = &spec.current_dir {
        command.current_dir(dir);
    }
    command.env("LC_ALL", "C").env("LANG", "C");
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for fd in 0..3 {
                if libc::dup2(slave_fd, fd) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            Ok(())
        });
    }
    // setsid() in pre_exec makes this process its own session, which is
    // also its process group, so Cancel can still signal the group.
    // Calling process_group() as well would make setsid fail.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|err| {
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
        Error::Failed(format!("Couldn't run {}: {err}", spec.program.display()))
    })?;
    unsafe {
        libc::close(slave);
    }
    cancel.register(child.id());
    let mut tty = unsafe { File::from_raw_fd(master) };
    unsafe {
        let flags = libc::fcntl(master, libc::F_GETFL);
        libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut prelude = Vec::new();
    let mut scratch = [0u8; 1024];
    for turn in 0..times.max(1) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match tty.read(&mut scratch) {
                Ok(0) | Err(_) => std::thread::sleep(Duration::from_millis(20)),
                Ok(n) => {
                    push_tail(&mut prelude, &scratch[..n]);
                    let text = String::from_utf8_lossy(&prelude).to_lowercase();
                    let ready = if turn == 0 {
                        text.contains("password") || text.contains("passphrase")
                    } else {
                        text.contains("verify")
                    };
                    if ready {
                        break;
                    }
                }
            }
        }
        let line = format!("{password}\n");
        let _ = tty.write_all(line.as_bytes());
        let _ = tty.flush();
    }

    let mut collected = Vec::new();
    let mut buf = [0u8; 4096];
    let start = std::time::Instant::now();
    loop {
        if cancel.is_cancelled() {
            cancel.cancel();
            break;
        }
        // A short non-blocking read, then see if the child has exited.
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        match tty.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => push_tail(&mut collected, &buf[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // Drain whatever is left.
                loop {
                    match tty.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => push_tail(&mut collected, &buf[..n]),
                    }
                }
                cancel.clear_pid();
                let text = String::from_utf8_lossy(&collected).into_owned();
                return Ok(Output {
                    code: status.code(),
                    text,
                });
            }
            Ok(None) if start.elapsed() > Duration::from_secs(120) => {
                let _ = child.kill();
                let _ = child.wait();
                cancel.clear_pid();
                return Err(Error::Failed("The archive tool took too long.".into()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                cancel.clear_pid();
                break;
            }
        }
    }
    cancel.clear_pid();
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let text = String::from_utf8_lossy(&collected).into_owned();
    Ok(Output { code: None, text })
}

pub fn volume_switch(bytes: u64) -> String {
    if bytes >= (1 << 30) && bytes % (1 << 30) == 0 {
        format!("-v{}g", bytes >> 30)
    } else if bytes >= (1 << 20) && bytes % (1 << 20) == 0 {
        format!("-v{}m", bytes >> 20)
    } else if bytes >= (1 << 10) && bytes % (1 << 10) == 0 {
        format!("-v{}k", bytes >> 10)
    } else {
        format!("-v{bytes}b")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_and_backspace() {
        let mut buf = Vec::new();
        apply_controls(&mut buf, b"  0%\x08\x08\x08\x08 12%");
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(last_percent(&text), Some(12));
    }

    #[test]
    fn volume_switch_uses_7zip_units() {
        assert_eq!(volume_switch(10 * 1024 * 1024), "-v10m");
        assert_eq!(volume_switch(64 * 1024), "-v64k");
        assert_eq!(volume_switch(1024 * 1024 * 1024), "-v1g");
        assert_eq!(volume_switch(1_457_664), "-v1457664b");
        assert_eq!(volume_switch(4_294_967_295), "-v4294967295b");
        assert_eq!(volume_switch(4_700_372_992), "-v4590208k");
    }

    #[test]
    fn a_terminal_password_is_not_an_argument() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("got");
        let script = format!(
            "IFS= read -r line < /dev/tty; printf %s \"$line\" > {}",
            out.display()
        );
        let ran = run_on_pty(
            Spec {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script],
                current_dir: None,
                writable: None,
                stdin: None,
                stdout_path: None,
            },
            "correct horse",
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(ran.code, Some(0), "{}", ran.text);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "correct horse");
    }
}
