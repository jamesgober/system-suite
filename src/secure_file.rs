//! Refusing a file another user can change, before it is read.

#[cfg(unix)]
use std::fs;
use std::path::Path;

#[cfg(not(unix))]
use crate::Error;
#[cfg(unix)]
use crate::{Error, ErrorKind};

/// The 0.2 defence idea (D-020): a file that another user can change is
/// refused before it is read. On Unix: not group- or world-writable, and
/// owned by the service's user or root (the owner check needs the `sys`
/// feature, which reads the current user). Windows files are governed by
/// their directory's access control list, which this check does not read.
#[cfg(unix)]
#[cfg_attr(not(feature = "env"), allow(dead_code))]
pub(crate) fn check(path: &Path, what: &str) -> Result<(), Error> {
    let metadata = fs::metadata(path).map_err(|e| uninspectable(path, what, &e))?;
    check_metadata(&metadata, path, what)
}

/// As [`check`], on a file already opened: the check applies to exactly
/// what is read, so swapping the file between the check and the read does
/// not work (the 0.4 defence idea, D-049).
#[cfg(unix)]
#[cfg_attr(not(feature = "config"), allow(dead_code))]
pub(crate) fn check_open(file: &fs::File, path: &Path, what: &str) -> Result<(), Error> {
    let metadata = file.metadata().map_err(|e| uninspectable(path, what, &e))?;
    check_metadata(&metadata, path, what)
}

#[cfg(unix)]
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
/// checked on the opened file. Windows files are governed by their
/// directory's access control list, which this check does not read.
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

#[cfg(all(not(unix), feature = "secrets"))]
pub(crate) fn check_secret_open(
    _file: &std::fs::File,
    _path: &Path,
    _key: &str,
) -> Result<(), Error> {
    Ok(())
}

#[cfg(not(unix))]
#[cfg_attr(not(feature = "env"), allow(dead_code))]
pub(crate) fn check(_path: &Path, _what: &str) -> Result<(), Error> {
    Ok(())
}

#[cfg(not(unix))]
#[cfg_attr(not(feature = "config"), allow(dead_code))]
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
