//! Refusing a file another account can change, before it is read.
//!
//! Every file the suite trusts (the configuration, the `.env` file, a
//! `file:` secret, an existing audit chain) is opened through
//! [`open_trusted`], which uses path-lib's `open_checked`: the owner and
//! permissions are read from the opened handle, so the file checked is the
//! file read (D-049), and the open does not block, so a FIFO planted where
//! the file should be is refused instead of hanging the boot (D-104). The
//! checks are the same on every OS (D-098): the owner must be the service's
//! account or an administrator account (root; `SYSTEM` or `Administrators`),
//! and no other account may change the file, or, for a secret, read it.

use std::path::Path;
#[cfg(feature = "paths")]
use std::{fs, io};

#[cfg(feature = "paths")]
use crate::{Error, ErrorKind};

#[cfg(feature = "paths")]
pub(crate) use path_lib::Others;

/// Why [`open_trusted`] did not return a file.
#[cfg(feature = "paths")]
#[derive(Debug)]
pub(crate) enum Refusal {
    /// Nothing exists at the path (or at the end of its links).
    NotFound,
    /// The file exists and is refused: path-lib's reason, which says what
    /// to fix.
    Refused(&'static str),
    /// It could not be opened or inspected.
    Io(String),
}

/// Opens `path` for reading after the owner and permission checks, made
/// on the returned handle.
///
/// Links are resolved first, and the file they lead to is checked and
/// opened: Kubernetes mounts configuration maps and secrets as links into
/// a hidden directory (`db_password -> ..data/db_password`), and a link is
/// only as trustworthy as the directory holding it, which belongs to the
/// operator (D-104). On Windows a file on a volume that keeps no access
/// control lists (FAT, exFAT) is accepted, since no check can protect it
/// and refusing it would break portable installs on such drives (D-098).
#[cfg(feature = "paths")]
#[cfg_attr(
    not(any(feature = "config", feature = "env", feature = "audit")),
    allow(dead_code)
)]
pub(crate) fn open_trusted(path: &Path, others: Others) -> Result<fs::File, Refusal> {
    let target = fs::canonicalize(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => Refusal::NotFound,
        _ => Refusal::Io(e.to_string()),
    })?;
    match path_lib::open_checked(&target, others) {
        Ok(file) => Ok(file),
        Err(path_lib::Error::NotFound { .. }) => Err(Refusal::NotFound),
        Err(path_lib::Error::Refused { reason, .. }) => {
            #[cfg(windows)]
            if let Some(file) = unenforced(&target) {
                return Ok(file);
            }
            Err(Refusal::Refused(reason))
        }
        Err(other) => Err(Refusal::Io(other.to_string())),
    }
}

/// The file at `target`, when it is a regular file on a volume without
/// access control lists.
#[cfg(all(windows, feature = "paths"))]
fn unenforced(target: &Path) -> Option<fs::File> {
    let file = fs::File::open(target).ok()?;
    let regular = file.metadata().ok()?.is_file();
    let access = path_lib::file_access(&file).ok()?;
    (regular && !access.enforced).then_some(file)
}

/// The error for a refused configuration-like file (`what` names it).
#[cfg(feature = "paths")]
#[cfg_attr(
    not(any(feature = "config", feature = "env", feature = "audit")),
    allow(dead_code)
)]
pub(crate) fn refused(path: &Path, what: &str, reason: &str) -> Error {
    Error::new(
        ErrorKind::ConfigInsecure,
        format!("the {what} {} is refused: {reason}", path.display()),
        FIX_OWNER,
    )
}

/// What to do about a file another account owns or can change.
#[cfg(all(feature = "paths", unix))]
pub(crate) const FIX_OWNER: &str =
    "chmod 640 (or 600) the file and chown it to the service's user (or root)";
#[cfg(all(feature = "paths", windows))]
pub(crate) const FIX_OWNER: &str = "make the service's account, SYSTEM, or Administrators its owner, and remove every other account's write access (icacls <file> /inheritance:r /grant:r <account>:F)";
#[cfg(all(feature = "paths", not(any(unix, windows))))]
pub(crate) const FIX_OWNER: &str =
    "make the file owned by the service's user and writable by it only";

/// What to do about a secret file another account can read or change.
#[cfg(all(feature = "secrets", unix))]
pub(crate) const FIX_SECRET: &str =
    "chmod 600 (or 400) the file and chown it to the service's user (or root)";
#[cfg(all(feature = "secrets", windows))]
pub(crate) const FIX_SECRET: &str = "make the service's account, SYSTEM, or Administrators its owner, and remove every other account's access (icacls <file> /inheritance:r /grant:r <account>:F)";
#[cfg(all(feature = "secrets", not(any(unix, windows))))]
pub(crate) const FIX_SECRET: &str = "make the file readable by the service's user only";

/// The largest `.env` file accepted.
#[cfg(feature = "env")]
const MAX_ENV_FILE: u64 = 1 << 20;

/// Reads the `.env` file for boot step 4, through [`open_trusted`]:
/// refused when it is not a regular file, when another account owns it or
/// can change it, when `others` is [`Others::NoAccess`] (a system-wide
/// install, where the file can hold `env:` secrets) and another account
/// can read it, or when it is larger than 1 MiB. The same rules as env-lib
/// 1.1's own `Env::open`.
#[cfg(feature = "env")]
pub(crate) fn read_env_file(path: &Path, others: Others) -> Result<String, Error> {
    use std::io::Read as _;

    let failed = |detail: String| {
        Error::new(
            ErrorKind::Environment,
            format!("the environment file {} {detail}", path.display()),
            "check that it is a regular file the service's user can read",
        )
    };
    let metadata =
        fs::metadata(path).map_err(|e| failed(format!("cannot be inspected ({})", e.kind())))?;
    if !metadata.is_file() {
        return Err(failed("is not a regular file".to_owned()));
    }
    let file = open_trusted(path, others).map_err(|refusal| match refusal {
        Refusal::Refused(reason) => refused(path, "environment file", reason),
        Refusal::NotFound => failed("does not exist".to_owned()),
        Refusal::Io(detail) => failed(format!("cannot be opened ({detail})")),
    })?;
    let mut text = String::new();
    let read = file
        .take(MAX_ENV_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|e| failed(format!("cannot be read ({})", e.kind())))?;
    if read as u64 > MAX_ENV_FILE {
        return Err(failed("is larger than 1 MiB".to_owned()));
    }
    Ok(text)
}

/// Makes `dir` owner-only on Windows for a unit test (the current user,
/// SYSTEM, and Administrators, nothing inherited): a machine's temporary
/// folder can grant other accounts write access, which the checks refuse.
#[cfg(all(test, windows, feature = "config"))]
pub(crate) fn make_private_for_tests(dir: &Path) {
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(domain), Ok(user)) => format!("{domain}\\{user}"),
        (_, Ok(user)) => user,
        _ => panic!("USERNAME is not set"),
    };
    let status = std::process::Command::new("icacls")
        .arg(dir)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{user}:(OI)(CI)F"))
        .args([
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
        ])
        .stdout(std::process::Stdio::null())
        .status();
    assert!(
        status.is_ok_and(|s| s.success()),
        "icacls failed on {}",
        dir.display()
    );
}

#[cfg(all(test, not(windows), feature = "config"))]
pub(crate) fn make_private_for_tests(_dir: &Path) {}
