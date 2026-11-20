//! The only module that knows the gocryptfs command line.
//!
//! Secrets are written to the child's stdin and never appear in argv or the
//! environment. Programs are called by absolute path with a cleared
//! environment, so a session service cannot be hijacked through `$PATH`.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::mountinfo::is_mounted;
use crate::paths::create_private_dir_all;
use crate::secret::{secret_from_vec, ExposeSecret, SecretBytes};

const MOUNT_TIMEOUT: Duration = Duration::from_secs(30);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(50);

/// gocryptfs exit codes (internal/exitcodes).
const EXIT_PASSWORD_INCORRECT: i32 = 12;
const EXIT_FUSE_NEW_SERVER: i32 = 19;

/// How to open a vault.
pub enum Unlock {
    Passphrase(SecretBytes),
    /// Master key as printed by `gocryptfs-xray -dumpmasterkey`.
    MasterKey(SecretBytes),
}

#[derive(Debug, Clone)]
pub struct Gocryptfs {
    gocryptfs: PathBuf,
    xray: PathBuf,
    fusermount: PathBuf,
}

impl Default for Gocryptfs {
    fn default() -> Self {
        Self::new("/usr/bin/gocryptfs", "/usr/bin/gocryptfs-xray", "/usr/bin/fusermount3")
    }
}

impl Gocryptfs {
    pub fn new(gocryptfs: impl Into<PathBuf>, xray: impl Into<PathBuf>, fusermount: impl Into<PathBuf>) -> Self {
        Self {
            gocryptfs: gocryptfs.into(),
            xray: xray.into(),
            fusermount: fusermount.into(),
        }
    }

    /// Whether this process can gain privileges through the setuid
    /// `fusermount3` (it cannot when `NoNewPrivs` is set, e.g. by seccomp
    /// options in a systemd user unit).
    pub fn can_mount() -> bool {
        std::fs::read_to_string("/proc/self/status")
            .map(|s| !s.lines().any(|l| l.starts_with("NoNewPrivs:") && l.trim_end().ends_with('1')))
            .unwrap_or(true)
    }

    /// Whether the gocryptfs binaries are installed.
    pub fn is_available(&self) -> bool {
        self.gocryptfs.is_file() && self.xray.is_file() && self.fusermount.is_file()
    }

    /// Creates a new vault in `cipher_dir` (created with mode 0700, must be empty).
    pub fn init(&self, cipher_dir: &Path, passphrase: &SecretBytes) -> Result<()> {
        check_passphrase(passphrase)?;
        create_private_dir_all(cipher_dir)?;
        if fs::read_dir(cipher_dir)?.next().is_some() {
            return Err(Error::NotEmpty(cipher_dir.to_owned()));
        }
        let mut child = self
            .command(&self.gocryptfs)
            .args(["-q", "-init", "-passfile", "/dev/stdin", "--"])
            .arg(cipher_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        feed(&mut child, passphrase.expose_secret())?;
        let out = child.wait_with_output()?;
        check_status("gocryptfs", out.status, &out.stderr)
    }

    /// Mounts `cipher_dir` on `mount_point` (created with mode 0700, must be
    /// empty) and returns once the file system is visible. gocryptfs runs in
    /// the foreground as a child of the caller; its log goes to the caller's
    /// stderr (the journal for a systemd unit).
    pub fn mount(&self, cipher_dir: &Path, mount_point: &Path, unlock: Unlock) -> Result<MountHandle> {
        create_private_dir_all(mount_point)?;
        if is_mounted(mount_point)? || fs::read_dir(mount_point)?.next().is_some() {
            return Err(Error::NotEmpty(mount_point.to_owned()));
        }
        let (flag, secret) = match &unlock {
            Unlock::Passphrase(p) => {
                check_passphrase(p)?;
                (["-passfile", "/dev/stdin"].as_slice(), p)
            }
            Unlock::MasterKey(k) => (["-masterkey=stdin"].as_slice(), k),
        };
        let mut child = self
            .command(&self.gocryptfs)
            .args(["-fg", "-q", "-nosyslog"])
            .args(flag)
            .arg("--")
            .arg(cipher_dir)
            .arg(mount_point)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut line = Zeroizing::new(secret.expose_secret().to_vec());
        line.push(b'\n');
        feed(&mut child, &line)?;
        drop(line);

        let deadline = Instant::now() + MOUNT_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait()? {
                check_status("gocryptfs", status, b"")?;
                return Err(Error::Tool {
                    program: "gocryptfs",
                    code: status.code(),
                    message: "exited before mounting".into(),
                });
            }
            if is_mounted(mount_point)? {
                return Ok(MountHandle {
                    child: Some(child),
                    mount_point: mount_point.to_owned(),
                    fusermount: self.fusermount.clone(),
                });
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::MountTimeout(MOUNT_TIMEOUT.as_secs()));
            }
            sleep(POLL);
        }
    }

    /// Returns the vault master key (used to enroll fingerprint unlock).
    pub fn dump_master_key(&self, cipher_dir: &Path, passphrase: &SecretBytes) -> Result<SecretBytes> {
        check_passphrase(passphrase)?;
        let mut child = self
            .command(&self.xray)
            .arg("-dumpmasterkey")
            .arg(cipher_dir.join("gocryptfs.conf"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut line = Zeroizing::new(passphrase.expose_secret().to_vec());
        line.push(b'\n');
        feed(&mut child, &line)?;
        drop(line);

        let mut stdout = Zeroizing::new(Vec::new());
        child.stdout.take().expect("piped").read_to_end(&mut stdout)?;
        let mut stderr = Vec::new();
        child.stderr.take().expect("piped").read_to_end(&mut stderr)?;
        let status = child.wait()?;
        if !status.success() {
            if String::from_utf8_lossy(&stderr).contains("Password incorrect") {
                return Err(Error::WrongPassphrase);
            }
            return Err(tool_error("gocryptfs-xray", status, &stderr));
        }
        let key: Vec<u8> = stdout
            .iter()
            .copied()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let key = secret_from_vec(key);
        let k = key.expose_secret();
        if k.len() < 64 || !k.iter().all(|c| c.is_ascii_hexdigit() || *c == b'-') {
            return Err(Error::Tool {
                program: "gocryptfs-xray",
                code: status.code(),
                message: "unexpected output".into(),
            });
        }
        Ok(key)
    }

    /// Unmounts a mount point this process has no handle for (e.g. left over
    /// by a crashed session service).
    pub fn unmount_path(&self, mount_point: &Path, lazy: bool) -> Result<()> {
        fusermount_unmount(&self.fusermount, mount_point, lazy)
    }

    fn command(&self, program: &Path) -> Command {
        let mut c = Command::new(program);
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::piped());
        c
    }
}

/// A mounted vault. Dropping it unmounts (lazily) and reaps gocryptfs.
#[derive(Debug)]
pub struct MountHandle {
    child: Option<Child>,
    mount_point: PathBuf,
    fusermount: PathBuf,
}

impl MountHandle {
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    /// Whether gocryptfs is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    /// Unmounts and reaps gocryptfs; calling it again is a no-op. Without
    /// `lazy`, fails with [`Error::Busy`] while files are open, and the vault
    /// stays mounted.
    pub fn unmount(&mut self, lazy: bool) -> Result<()> {
        let Some(child) = self.child.as_mut() else { return Ok(()) };
        if child.try_wait()?.is_none() && is_mounted(&self.mount_point)? {
            fusermount_unmount(&self.fusermount, &self.mount_point, lazy)?;
        }
        let deadline = Instant::now() + EXIT_TIMEOUT;
        while child.try_wait()?.is_none() {
            if Instant::now() > deadline {
                child.kill()?;
                break;
            }
            sleep(POLL);
        }
        let _ = child.wait();
        self.child = None;
        Ok(())
    }
}

impl Drop for MountHandle {
    fn drop(&mut self) {
        let _ = self.unmount(true);
    }
}

fn fusermount_unmount(fusermount: &Path, mount_point: &Path, lazy: bool) -> Result<()> {
    let mut c = Command::new(fusermount);
    c.env_clear().env("PATH", "/usr/bin:/bin").env("LC_ALL", "C").arg("-u");
    if lazy {
        c.arg("-z");
    }
    let out = c.arg("--").arg(mount_point).stdin(Stdio::null()).output()?;
    if out.status.success() {
        return Ok(());
    }
    if String::from_utf8_lossy(&out.stderr).contains("busy") {
        return Err(Error::Busy);
    }
    Err(tool_error("fusermount3", out.status, &out.stderr))
}

/// gocryptfs `-passfile` only reads the first line.
fn check_passphrase(p: &SecretBytes) -> Result<()> {
    let p = p.expose_secret();
    if p.is_empty() {
        return Err(Error::InvalidPassphrase("empty"));
    }
    if p.contains(&b'\n') || p.contains(&b'\r') || p.contains(&0) {
        return Err(Error::InvalidPassphrase("contains a line break or NUL"));
    }
    Ok(())
}

/// Writes `data` to the child's stdin and closes it. A child that exits
/// early (broken pipe) is reported through its exit status instead.
fn feed(child: &mut Child, data: &[u8]) -> Result<()> {
    let mut stdin = child.stdin.take().expect("stdin is piped");
    match stdin.write_all(data) {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}

fn check_status(program: &'static str, status: ExitStatus, stderr: &[u8]) -> Result<()> {
    match status.code() {
        Some(0) => Ok(()),
        Some(EXIT_PASSWORD_INCORRECT) => Err(Error::WrongPassphrase),
        Some(EXIT_FUSE_NEW_SERVER) => Err(Error::Tool {
            program,
            code: status.code(),
            message: "could not mount the folder (FUSE). Is fusermount3 installed and allowed to run \
                      (setuid, no NoNewPrivileges)?"
                .into(),
        }),
        _ => Err(tool_error(program, status, stderr)),
    }
}

fn tool_error(program: &'static str, status: ExitStatus, stderr: &[u8]) -> Error {
    let message = String::from_utf8_lossy(stderr).trim().lines().last().unwrap_or("").to_owned();
    Error::Tool {
        program,
        code: status.code(),
        message,
    }
}
