//! Process exit codes.
//!
//! The codes follow BSD `sysexits.h` so supervisors and scripts can tell a
//! configuration error from a second copy or a crash; `79` is the one code
//! of the suite's own (D-017). A panic ends the process as the application's
//! profile decides (with `panic = "abort"`, SIGABRT, shown as 134 by shells
//! on Unix).
//!
//! # Examples
//!
//! ```
//! use system_suite::exit;
//!
//! assert_eq!(exit::TEMPFAIL, 75);
//! let code = std::process::ExitCode::from(exit::CONFIG);
//! # let _ = code;
//! ```

/// A clean stop, whatever requested it.
pub const OK: u8 = 0;
/// An invalid setup in the code (`EX_USAGE`).
pub const USAGE: u8 = 64;
/// The audit chain failed verification (`EX_DATAERR`).
pub const DATAERR: u8 = 65;
/// A requirement of the environment is missing (`EX_UNAVAILABLE`).
pub const UNAVAILABLE: u8 = 69;
/// An internal failure, a subsystem failure, or a critical worker failure
/// (`EX_SOFTWARE`).
pub const SOFTWARE: u8 = 70;
/// A directory or file cannot be created (`EX_CANTCREAT`).
pub const CANTCREAT: u8 = 73;
/// An I/O error (`EX_IOERR`).
pub const IOERR: u8 = 74;
/// Already running: try again later (`EX_TEMPFAIL`).
pub const TEMPFAIL: u8 = 75;
/// Insecure permissions were refused (`EX_NOPERM`).
pub const NOPERM: u8 = 77;
/// The configuration is missing or invalid (`EX_CONFIG`).
pub const CONFIG: u8 = 78;
/// The shutdown deadline was missed and something was abandoned.
pub const DEADLINE: u8 = 79;
