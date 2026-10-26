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
/// owned by the service's user or root. Windows files are governed by
/// their directory's access control list, which this check does not read.
#[cfg(unix)]
pub(crate) fn check(path: &Path, what: &str) -> Result<(), Error> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = fs::metadata(path).map_err(|e| {
        Error::new(
            ErrorKind::ConfigInvalid,
            format!("the {what} {} cannot be inspected: {e}", path.display()),
            "check that the service's user can read it",
        )
    })?;
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

#[cfg(not(unix))]
pub(crate) fn check(_path: &Path, _what: &str) -> Result<(), Error> {
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
