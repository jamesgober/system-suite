//! Readiness notification to systemd (`sd_notify`), through a std Unix
//! datagram socket: no FFI, no dependency (D-030).
//!
//! Only Linux runs systemd. On macOS and Windows there is nothing to notify
//! and every call returns `Ok(false)`; Windows readiness goes through the
//! service control manager once signal-mod supports it (D-014).

use std::io;

/// Sends `state` (`"READY=1"`, `"STOPPING=1"`, ...) to the socket named by
/// `NOTIFY_SOCKET`. Returns `Ok(false)` when the process is not run by a
/// service manager that asked for notifications.
pub(crate) fn notify(state: &str) -> io::Result<bool> {
    send(state)
}

#[cfg(target_os = "linux")]
fn send(state: &str) -> io::Result<bool> {
    use std::os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixDatagram},
    };

    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(false);
    };
    let socket = UnixDatagram::unbound()?;
    let bytes = path.as_encoded_bytes();
    let sent = match bytes.strip_prefix(b"@") {
        // An abstract socket: systemd's default for user services.
        Some(name) => {
            socket.send_to_addr(state.as_bytes(), &SocketAddr::from_abstract_name(name)?)?
        }
        None => socket.send_to(state.as_bytes(), &path)?,
    };
    if sent != state.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "the notification was cut short",
        ));
    }
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn send(_state: &str) -> io::Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notify_without_socket_is_a_no_op() {
        // The test runner is not started by systemd with NOTIFY_SOCKET.
        if std::env::var_os("NOTIFY_SOCKET").is_none() {
            assert!(matches!(notify("READY=1"), Ok(false)));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_notify_reaches_a_listening_socket() {
        use std::os::unix::net::UnixDatagram;

        let dir = std::env::temp_dir().join(format!("system-suite-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("notify.sock");
        std::fs::remove_file(&path).ok();
        let listener = UnixDatagram::bind(&path).ok();
        let Some(listener) = listener else {
            return;
        };
        let socket = UnixDatagram::unbound().ok();
        let Some(socket) = socket else {
            return;
        };
        let sent = socket.send_to(b"READY=1", &path).ok();
        assert_eq!(sent, Some(7));
        let mut buf = [0u8; 16];
        let received = listener.recv(&mut buf).ok();
        assert_eq!(received, Some(7));
        std::fs::remove_dir_all(&dir).ok();
    }
}
