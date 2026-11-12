//! Refusing a file another user can change, before it is read.

#[cfg(any(unix, all(windows, feature = "paths")))]
use std::fs;
use std::path::Path;

#[cfg(not(any(unix, all(windows, feature = "paths"))))]
use crate::Error;
#[cfg(any(unix, all(windows, feature = "paths")))]
use crate::{Error, ErrorKind};

/// The 0.2 defence idea (D-020): a file that another user can change is
/// refused before it is read. On Unix: not group- or world-writable, and
/// owned by the service's user or root (the owner check needs the `sys`
/// feature, which reads the current user). Checked on the opened file, so
/// swapping the file between the check and the read does not work (the 0.4
/// defence idea, D-049). On Windows the opened file's owner and access
/// control list are read through path-lib instead (D-098).
#[cfg(unix)]
#[cfg_attr(
    not(any(feature = "config", feature = "env", feature = "audit")),
    allow(dead_code)
)]
pub(crate) fn check_open(file: &fs::File, path: &Path, what: &str) -> Result<(), Error> {
    let metadata = file.metadata().map_err(|e| uninspectable(path, what, &e))?;
    check_metadata(&metadata, path, what)
}

#[cfg(any(unix, all(windows, feature = "paths")))]
fn uninspectable(path: &Path, what: &str, error: &std::io::Error) -> Error {
    Error::new(
        ErrorKind::ConfigInvalid,
        format!("the {what} {} cannot be inspected: {error}", path.display()),
        "check that the service's user can read it",
    )
}

#[cfg(unix)]
fn check_metadata(metadata: &fs::Metadata, path: &Path, what: &str) -> Result<(), Error> {
    use std::os::unix::fs::MetadataExt as _;

    if !metadata.is_file() {
        return Err(Error::new(
            ErrorKind::ConfigInvalid,
            format!("the {what} {} is not a regular file", path.display()),
            "point the setting at a regular file",
        ));
    }
    let mode = metadata.mode();
    if mode & 0o022 != 0 {
        return Err(Error::new(
            ErrorKind::ConfigInsecure,
            format!(
                "the {what} {} can be changed by other users (mode {:o})",
                path.display(),
                mode & 0o777
            ),
            "chmod 640 (or 600) the file so only its owner can change it",
        ));
    }
    if let Some(user) = current_uid() {
        let owner = metadata.uid();
        if owner != user && owner != 0 {
            return Err(Error::new(
                ErrorKind::ConfigInsecure,
                format!(
                    "the {what} {} belongs to another user (uid {owner})",
                    path.display()
                ),
                "chown the file to the service's user (or root)",
            ));
        }
    }
    Ok(())
}

/// A secret file (`file:` reference): owner-only (no group or other bits
/// at all, `0o600` or stricter) and owned by the service's user or root,
/// checked on the opened file. On Windows: owned by the service's account,
/// SYSTEM, or Administrators, and no other account may read or change it
/// (D-098).
#[cfg(all(unix, feature = "secrets"))]
pub(crate) fn check_secret_open(file: &fs::File, path: &Path, key: &str) -> Result<(), Error> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file.metadata().map_err(|e| {
        Error::new(
            ErrorKind::Secret,
            format!(
                "the secret file for `{key}` cannot be inspected ({})",
                e.kind()
            ),
            "check that the service's user can read it",
        )
    })?;
    if !metadata.is_file() {
        return Err(Error::new(
            ErrorKind::Secret,
            format!(
                "the secret file {} for `{key}` is not a regular file",
                path.display()
            ),
            "point the reference at a regular file",
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(Error::new(
            ErrorKind::Secret,
            format!(
                "the secret file {} for `{key}` can be read or changed by other users (mode {:o})",
                path.display(),
                metadata.mode() & 0o777
            ),
            "chmod 600 (or 400) the file",
        ));
    }
    if let Some(user) = current_uid() {
        let owner = metadata.uid();
        if owner != user && owner != 0 {
            return Err(Error::new(
                ErrorKind::Secret,
                format!(
                    "the secret file {} for `{key}` belongs to another user (uid {owner})",
                    path.display()
                ),
                "chown the file to the service's user (or root)",
            ));
        }
    }
    Ok(())
}

#[cfg(all(windows, feature = "secrets"))]
pub(crate) fn check_secret_open(file: &fs::File, path: &Path, key: &str) -> Result<(), Error> {
    let what = format!("secret file for `{key}`");
    let secret = |error: Error| error.with_kind(ErrorKind::Secret);
    let access = inspect(file, path, &what).map_err(secret)?;
    let Some(access) = access else {
        return Ok(());
    };
    refuse_others(&access, path, &what).map_err(secret)?;
    if access.others_can_read {
        return Err(Error::new(
            ErrorKind::Secret,
            format!(
                "the secret file {} for `{key}` can be read by other accounts",
                path.display()
            ),
            "remove read access for every account but the service's, SYSTEM, and Administrators (icacls <file> /inheritance:r /grant:r <account>:F)",
        ));
    }
    Ok(())
}

#[cfg(all(not(any(unix, windows)), feature = "secrets"))]
pub(crate) fn check_secret_open(
    _file: &std::fs::File,
    _path: &Path,
    _key: &str,
) -> Result<(), Error> {
    Ok(())
}

/// On Windows: the opened file is a regular file, owned by the service's
/// account, SYSTEM, or Administrators, and no other account may change it
/// (D-098). A volume that keeps no access control lists (FAT, exFAT) cannot
/// be checked, and its files are accepted: anyone with the volume can
/// change any file on it, which no check can fix.
#[cfg(all(windows, feature = "paths"))]
#[cfg_attr(
    not(any(feature = "config", feature = "env", feature = "audit")),
    allow(dead_code)
)]
pub(crate) fn check_open(file: &fs::File, path: &Path, what: &str) -> Result<(), Error> {
    match inspect(file, path, what)? {
        Some(access) => refuse_others(&access, path, what),
        None => Ok(()),
    }
}

/// The opened file's access, or `None` on a volume without access control
/// lists. Refuses what is not a regular file.
#[cfg(all(windows, feature = "paths"))]
fn inspect(
    file: &fs::File,
    path: &Path,
    what: &str,
) -> Result<Option<path_lib::FileAccess>, Error> {
    let metadata = file.metadata().map_err(|e| uninspectable(path, what, &e))?;
    if !metadata.is_file() {
        return Err(Error::new(
            ErrorKind::ConfigInvalid,
            format!("the {what} {} is not a regular file", path.display()),
            "point the setting at a regular file",
        ));
    }
    let access = path_lib::file_access(file).map_err(|e| uninspectable(path, what, &e))?;
    Ok(access.enforced.then_some(access))
}

/// Refuses a file another account owns or can change.
#[cfg(all(windows, feature = "paths"))]
fn refuse_others(access: &path_lib::FileAccess, path: &Path, what: &str) -> Result<(), Error> {
    if !access.owner_trusted {
        return Err(Error::new(
            ErrorKind::ConfigInsecure,
            format!("the {what} {} belongs to another account", path.display()),
            "make the service's account, SYSTEM, or Administrators its owner (icacls <file> /setowner <account>)",
        ));
    }
    if access.others_can_write {
        return Err(Error::new(
            ErrorKind::ConfigInsecure,
            format!(
                "the {what} {} can be changed by other accounts",
                path.display()
            ),
            "remove write access for every account but the service's, SYSTEM, and Administrators (icacls <file> /inheritance:r /grant:r <account>:F)",
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, all(windows, feature = "paths"))))]
#[cfg_attr(
    not(any(feature = "config", feature = "env", feature = "audit")),
    allow(dead_code)
)]
pub(crate) fn check_open(_file: &std::fs::File, _path: &Path, _what: &str) -> Result<(), Error> {
    Ok(())
}

#[cfg(all(unix, feature = "sys"))]
fn current_uid() -> Option<u32> {
    sys_lib::process::user().ok().and_then(|user| user.id)
}

#[cfg(all(unix, not(feature = "sys")))]
fn current_uid() -> Option<u32> {
    None
}

/// The largest `.env` file accepted.
#[cfg(feature = "env")]
const MAX_ENV_FILE: u64 = 1 << 20;

/// Reads the `.env` file for boot step 4: refused when it is not a regular
/// file (checked before opening, so a FIFO cannot block the boot), when
/// another user can change it (checked on the opened file, like the
/// configuration file, D-049), or when it is larger than 1 MiB.
#[cfg(feature = "env")]
pub(crate) fn read_env_file(path: &Path) -> Result<String, Error> {
    use std::io::Read as _;

    use crate::ErrorKind;

    let failed = |detail: String| {
        Error::new(
            ErrorKind::Environment,
            format!("the environment file {} {detail}", path.display()),
            "check that it is a regular file the service's user can read",
        )
    };
    let metadata = std::fs::metadata(path)
        .map_err(|e| failed(format!("cannot be inspected ({})", e.kind())))?;
    if !metadata.is_file() {
        return Err(failed("is not a regular file".to_owned()));
    }
    let file = std::fs::File::open(path)
        .map_err(|e| failed(format!("cannot be opened ({})", e.kind())))?;
    check_open(&file, path, "environment file")?;
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
