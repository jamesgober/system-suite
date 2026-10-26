//! The suite's view of the configuration: one file per application, one
//! section per member, read through config-lib (D-034) in its `conf` format.
//!
//! The view does not depend on the backend: a later format (NOML or TOML)
//! or a change of backend changes the accepted file syntax, not these
//! signatures (D-018). config-lib itself is re-exported as
//! [`backend`] for anything the view does not cover.
//!
//! ```conf
//! # site.conf
//! [log]
//! level = "debug"
//!
//! [kernel]
//! shutdown_deadline = "40s"
//!
//! [http]
//! port = 8080
//! hosts = "a.example", "b.example"
//! ```
//!
//! In the `conf` format a value with spaces must be quoted: an unquoted
//! `reason = disk full` is read as the array `["disk", "full"]`.

use std::{fmt, path::Path, sync::Arc};

use crate::{Error, ErrorKind};

pub(crate) mod load;
#[cfg(feature = "kernel")]
pub(crate) mod reload;
pub(crate) mod sections;

/// config-lib, the configuration backend (D-034).
pub use config_lib as backend;

/// An immutable snapshot of the configuration.
///
/// Keys are dotted paths (`"database.pool.size"`). [`get`](Self::get)
/// returns `Ok(None)` for an absent key and an error of kind
/// [`ConfigInvalid`](ErrorKind::ConfigInvalid) (naming the key, the type
/// expected, and the type found, never the value) for a present key of the
/// wrong type. `Send + Sync`; cloning is one reference-count increment. A
/// handler that wants the newest configuration on every request keeps a
/// [`Reader`](crate::Reader) from `Foundation::config_snapshot()`.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::config::Config;
///
/// let config = Config::from_text("[http]\nport = 8080\nread_timeout = \"30s\"\n")?;
/// let port: u16 = config.get_or("http.port", 80)?;
/// let timeout: Duration = config.require("http.read_timeout")?;
/// assert_eq!((port, timeout), (8080, Duration::from_secs(30)));
/// assert_eq!(config.get::<String>("http.missing")?, None);
/// assert!(config.get::<bool>("http.port").is_err());   // wrong type
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct Config {
    inner: Arc<Inner>,
}

struct Inner {
    document: config_lib::Config,
    source: Option<std::path::PathBuf>,
    version: u64,
}

impl Config {
    /// Parses configuration text in the `conf` format, as
    /// `Builder::config_str` does, without the suite's section checks.
    /// For tests and tools; a running system's configuration is
    /// `Foundation::config()`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ConfigInvalid`] when the text does not parse.
    pub fn from_text(text: &str) -> Result<Config, Error> {
        load::parse(text, None, 1)
    }

    pub(crate) fn new(document: config_lib::Config, source: Option<&Path>, version: u64) -> Config {
        Config {
            inner: Arc::new(Inner {
                document,
                source: source.map(Path::to_path_buf),
                version,
            }),
        }
    }

    /// The value at `path`, converted to `T`; `Ok(None)` when absent.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ConfigInvalid`] when the value has the wrong type or is
    /// out of `T`'s range.
    pub fn get<T: FromConfig>(&self, path: &str) -> Result<Option<T>, Error> {
        match self.value(path) {
            Some(value) => T::from_value(value, path).map(Some),
            None => Ok(None),
        }
    }

    /// The value at `path`, which must be present.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ConfigInvalid`] when it is absent or has the wrong type.
    pub fn require<T: FromConfig>(&self, path: &str) -> Result<T, Error> {
        self.get(path)?.ok_or_else(|| {
            Error::new(
                ErrorKind::ConfigInvalid,
                format!("the configuration key `{path}` is required but missing"),
                format!("add `{path}` to the configuration file"),
            )
        })
    }

    /// The value at `path`, or `default` when absent.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ConfigInvalid`] when it is present with the wrong type.
    pub fn get_or<T: FromConfig>(&self, path: &str, default: T) -> Result<T, Error> {
        Ok(self.get(path)?.unwrap_or(default))
    }

    /// Whether a value is present at `path`.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.value(path).is_some()
    }

    /// The keys of `section`, sorted; empty when it is absent or not a
    /// section.
    #[must_use]
    pub fn keys(&self, section: &str) -> Vec<&str> {
        match self.value(section) {
            Some(config_lib::Value::Table(table)) => table.keys().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }

    /// The file it was read from; `None` for `Builder::config_str`, text,
    /// or defaults only.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.inner.source.as_deref()
    }

    /// 1 at boot, plus one per applied reload.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.inner.version
    }

    /// config-lib's own document, for anything this view does not cover.
    #[must_use]
    pub fn backend(&self) -> &backend::Config {
        &self.inner.document
    }

    pub(crate) fn value(&self, path: &str) -> Option<&config_lib::Value> {
        self.inner.document.get(path)
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the values: they may hold anything.
        f.debug_struct("Config")
            .field("source", &self.inner.source)
            .field("version", &self.inner.version)
            .finish_non_exhaustive()
    }
}

mod private {
    pub trait Sealed {}
}

/// Types a configuration value converts to.
///
/// Implemented for `bool`, `i8` to `i64`, `u8` to `u64`, `usize`, `f64`,
/// `String`, `PathBuf` (a leading `~` expanded through path-lib),
/// `Duration` (an integer number of seconds, or a string with a unit:
/// `"500ms"`, `"30s"`, `"5m"`, `"2h"`), and `Vec<T>` of any of these.
/// Integers are range-checked into the target type. The trait is sealed so
/// the suite can change the backend without breaking implementations;
/// structured values are read field by field, or through
/// [`Config::backend`].
pub trait FromConfig: Sized + private::Sealed {
    /// Converts the backend's value at `path`.
    #[doc(hidden)]
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error>;
}

fn wrong(path: &str, expected: &str, value: &config_lib::Value) -> Error {
    Error::new(
        ErrorKind::ConfigInvalid,
        format!(
            "the configuration key `{path}` should be {expected}, but it is {}",
            article(value.type_name())
        ),
        format!("change `{path}` to {expected}"),
    )
}

fn article(type_name: &str) -> String {
    match type_name {
        "integer" | "array" => format!("an {type_name}"),
        other => format!("a {other}"),
    }
}

impl private::Sealed for bool {}
impl FromConfig for bool {
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
        match value {
            config_lib::Value::Bool(b) => Ok(*b),
            other => Err(wrong(path, "true or false", other)),
        }
    }
}

macro_rules! integers {
    ($($t:ty),*) => {$(
        impl private::Sealed for $t {}
        impl FromConfig for $t {
            fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
                let expected = || format!("an integer from {} to {}", <$t>::MIN, <$t>::MAX);
                match value {
                    config_lib::Value::Integer(i) => <$t>::try_from(*i).map_err(|_| {
                        Error::new(
                            ErrorKind::ConfigInvalid,
                            format!("the configuration key `{path}` is out of range for {}", expected()),
                            format!("change `{path}` to {}", expected()),
                        )
                    }),
                    other => Err(wrong(path, &expected(), other)),
                }
            }
        }
    )*};
}

integers!(i8, i16, i32, i64, u8, u16, u32, u64, usize);

impl private::Sealed for f64 {}
impl FromConfig for f64 {
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
        match value {
            config_lib::Value::Float(f) => Ok(*f),
            config_lib::Value::Integer(i) => Ok(*i as f64),
            other => Err(wrong(path, "a number", other)),
        }
    }
}

impl private::Sealed for String {}
impl FromConfig for String {
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
        match value {
            config_lib::Value::String(s) => Ok(s.clone()),
            other => Err(wrong(path, "a string", other)),
        }
    }
}

impl private::Sealed for std::path::PathBuf {}
impl FromConfig for std::path::PathBuf {
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
        let text = String::from_value(value, path)?;
        expand_home(&text, path)
    }
}

#[cfg(feature = "paths")]
fn expand_home(text: &str, key: &str) -> Result<std::path::PathBuf, Error> {
    if text.starts_with('~') {
        path_lib::expand_home(text).map_err(|e| {
            Error::new(
                ErrorKind::ConfigInvalid,
                format!("the configuration key `{key}` holds a path that cannot be expanded: {e}"),
                format!("change `{key}` to an absolute path or `~/...`"),
            )
        })
    } else {
        Ok(std::path::PathBuf::from(text))
    }
}

#[cfg(not(feature = "paths"))]
fn expand_home(text: &str, _key: &str) -> Result<std::path::PathBuf, Error> {
    Ok(std::path::PathBuf::from(text))
}

impl private::Sealed for std::time::Duration {}
impl FromConfig for std::time::Duration {
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
        let expected =
            "a duration: whole seconds, or a string such as \"500ms\", \"30s\", \"5m\", \"2h\"";
        match value {
            config_lib::Value::Integer(i) => u64::try_from(*i)
                .map(std::time::Duration::from_secs)
                .map_err(|_| wrong(path, expected, value)),
            config_lib::Value::String(s) => parse_duration(s).ok_or_else(|| {
                Error::new(
                    ErrorKind::ConfigInvalid,
                    format!("the configuration key `{path}` is not {expected}"),
                    format!("change `{path}` to {expected}"),
                )
            }),
            other => Err(wrong(path, expected, other)),
        }
    }
}

/// `"500ms"`, `"30s"`, `"5m"`, `"2h"`: a whole number and a unit.
pub(crate) fn parse_duration(text: &str) -> Option<std::time::Duration> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().ok()?;
    let millis_per_unit: u64 = match unit.trim() {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    number
        .checked_mul(millis_per_unit)
        .map(std::time::Duration::from_millis)
}

impl<T: FromConfig> private::Sealed for Vec<T> {}
impl<T: FromConfig> FromConfig for Vec<T> {
    fn from_value(value: &config_lib::Value, path: &str) -> Result<Self, Error> {
        match value {
            config_lib::Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, item)| T::from_value(item, &format!("{path}[{i}]")))
                .collect(),
            // A single value where a list is expected is a list of one.
            other => T::from_value(other, path).map(|item| vec![item]),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;

    fn config(text: &str) -> Config {
        Config::from_text(text).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn test_get_converts_scalars() {
        let c = config("[s]\nflag = true\nn = 42\nf = 1.5\nname = \"site\"\n");
        assert_eq!(c.get::<bool>("s.flag").ok().flatten(), Some(true));
        assert_eq!(c.get::<u16>("s.n").ok().flatten(), Some(42));
        assert_eq!(c.get::<f64>("s.f").ok().flatten(), Some(1.5));
        assert_eq!(c.get::<f64>("s.n").ok().flatten(), Some(42.0));
        assert_eq!(
            c.get::<String>("s.name").ok().flatten().as_deref(),
            Some("site")
        );
    }

    #[test]
    fn test_get_absent_is_none_and_require_fails() {
        let c = config("[s]\nn = 1\n");
        assert!(matches!(c.get::<u32>("s.missing"), Ok(None)));
        assert_eq!(
            c.require::<u32>("s.missing").err().map(|e| e.kind()),
            Some(ErrorKind::ConfigInvalid)
        );
        assert_eq!(c.get_or("s.missing", 7_u32).ok(), Some(7));
    }

    #[test]
    fn test_wrong_type_and_range_name_the_key_not_the_value() {
        let c = config("[s]\nsecret_ish = \"hunter2\"\nbig = 70000\n");
        let error = c.get::<u32>("s.secret_ish").err();
        let text = error.as_ref().map(ToString::to_string).unwrap_or_default();
        assert!(
            text.contains("s.secret_ish") && !text.contains("hunter2"),
            "{text}"
        );
        assert!(c.get::<u16>("s.big").is_err());
        assert!(c.get::<i8>("s.big").is_err());
        assert!(c.get::<u64>("s.big").is_ok());
    }

    #[test]
    fn test_durations_take_seconds_or_units() {
        let c = config("[k]\na = 30\nb = \"500ms\"\nc = \"5m\"\nd = \"2h\"\ne = \"soon\"\n");
        assert_eq!(
            c.get::<Duration>("k.a").ok().flatten(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            c.get::<Duration>("k.b").ok().flatten(),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            c.get::<Duration>("k.c").ok().flatten(),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            c.get::<Duration>("k.d").ok().flatten(),
            Some(Duration::from_secs(7_200))
        );
        assert!(c.get::<Duration>("k.e").is_err());
    }

    #[test]
    fn test_parse_duration_edges() {
        assert_eq!(parse_duration("0s"), Some(Duration::ZERO));
        assert_eq!(parse_duration("10"), None);
        assert_eq!(parse_duration("s"), None);
        assert_eq!(parse_duration("-5s"), None);
        assert_eq!(parse_duration(&format!("{}h", u64::MAX)), None);
    }

    #[test]
    fn test_lists_accept_arrays_and_single_values() {
        let c = config("[l]\nmany = [\"a\", \"b\", \"c\"]\none = \"x\"\n");
        let many: Vec<String> = c.require("l.many").unwrap_or_default();
        assert_eq!(many, vec!["a", "b", "c"]);
        let one: Vec<String> = c.require("l.one").unwrap_or_default();
        assert_eq!(one, vec!["x"]);
    }

    #[test]
    fn test_paths_and_keys_and_metadata() {
        let c = config("[p]\nroot = \"/var/lib/site\"\n[q]\nb = 1\na = 2\n");
        assert_eq!(
            c.get::<PathBuf>("p.root").ok().flatten(),
            Some(PathBuf::from("/var/lib/site"))
        );
        assert_eq!(c.keys("q"), vec!["a", "b"]);
        assert!(c.keys("absent").is_empty());
        assert!(c.contains("q.a") && !c.contains("q.z"));
        assert_eq!(c.version(), 1);
        assert!(c.source().is_none());
    }

    #[test]
    fn test_debug_never_shows_values() {
        let c = config("[db]\npassword = \"hunter2\"\n");
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
