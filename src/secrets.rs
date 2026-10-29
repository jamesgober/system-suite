//! Secret references in the configuration, resolved into key-vault (boot
//! step 8, and again on reload): `env:NAME`, `file:/path`, `vault:name`.
//!
//! Resolution is two-phase like the reload it is part of: [`resolve`]
//! reads every referenced value into wiped-on-drop buffers and changes
//! nothing; [`Resolved::install`] then puts them into the vault. A
//! failure in the first phase leaves the vault as it was.

use std::{
    collections::HashMap,
    fmt, fs,
    io::Read as _,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
};

use key_vault::{KeyHandle, KeyVault, RawKey};

use crate::{Error, ErrorKind, config::Config};

/// The largest secret file accepted.
const MAX_SECRET_FILE: u64 = 64 * 1024;

/// The last segments (or `_`-suffixes) of a key that mark it as holding a
/// secret: such a key must hold a reference, not a plain value.
const SECRET_WORDS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "credentials",
    "client_secret",
    "secret_key",
    "access_key",
];

/// The vault name a resolved value is stored under: per configuration
/// version, so a new version never changes what an older one reads.
fn vault_name(version: u64, path: &str) -> String {
    format!("config:v{version}:{path}")
}

/// The secrets of one configuration version: key path to vault handle.
pub(crate) struct Table {
    vault: KeyVault,
    handles: HashMap<String, KeyHandle>,
    /// The handles this table registered (not the application's `vault:`
    /// keys), removed (and wiped) when the table is retired.
    owned: Vec<KeyHandle>,
}

impl Table {
    /// Removes the values this table stored; the vault wipes them.
    pub(crate) fn retire(&self) {
        for handle in &self.owned {
            let _removed = self.vault.unregister(*handle);
        }
    }

    pub(crate) fn secret(&self, path: &str) -> Result<Secret, Error> {
        match self.handles.get(path) {
            Some(handle) => Ok(Secret {
                name: path.to_owned(),
                handle: *handle,
                vault: self.vault.clone(),
            }),
            None => Err(Error::new(
                ErrorKind::Secret,
                format!("the configuration key `{path}` holds no secret reference"),
                "write the key as env:NAME, file:/path, or vault:name",
            )),
        }
    }
}

/// A secret from the configuration: its key's name and its place in the
/// vault. The bytes are only visible inside [`with`](Self::with), through
/// key-vault's scoped buffer that is wiped afterwards. `Debug` prints the
/// name, never the value.
///
/// # Examples
///
/// ```no_run
/// # #[cfg(feature = "secrets")] {
/// // site.conf:  [database]  password = "file:/run/secrets/db_password"
/// let system = system_suite::start!()?;
/// let password = system.config().secret("database.password")?;
/// let length = password.with(|bytes| bytes.len())?;
/// # let _ = length;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct Secret {
    name: String,
    handle: KeyHandle,
    vault: KeyVault,
}

impl Secret {
    /// The configuration key the secret came from (`"database.password"`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Calls `use_it` with the secret's bytes in a buffer that is wiped
    /// when it returns (or panics). Copies `use_it` makes are not wiped.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Secret`] when the vault refuses (the key was removed,
    /// or the vault is locked out).
    pub fn with<T>(&self, use_it: impl FnOnce(&[u8]) -> T) -> Result<T, Error> {
        self.vault.with_key(self.handle, use_it).map_err(|e| {
            Error::new(
                ErrorKind::Secret,
                format!(
                    "the secret `{}` cannot be read from the vault: {e}",
                    self.name
                ),
                "check that the vault still holds the key",
            )
        })
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({:?}, [redacted])", self.name)
    }
}

/// Where a resolved value goes.
enum Value {
    /// Read from the environment or a file; stored at install.
    Raw(RawKey),
    /// Already in the vault, registered by the application.
    InVault(KeyHandle),
}

/// Every reference of one configuration, read but not yet stored.
pub(crate) struct Resolved {
    values: Vec<(String, Value)>,
}

/// What resolving needs besides the configuration.
pub(crate) struct Sources<'a> {
    pub(crate) vault: &'a KeyVault,
    #[cfg(feature = "env")]
    pub(crate) env: Option<&'a env_lib::Env>,
    pub(crate) allow_plain: bool,
}

/// Reads every secret reference of `config` (first phase): nothing is
/// stored, and a failure names the key, never the value.
pub(crate) fn resolve(config: &Config, sources: &Sources<'_>) -> Result<Resolved, Error> {
    let mut found = Vec::new();
    collect(config.backend().as_value(), &mut Vec::new(), &mut found);
    let mut values = Vec::with_capacity(found.len());
    for (path, text) in found {
        match reference(&text) {
            Some(("env", name)) => {
                values.push((path.clone(), Value::Raw(from_env(&path, name, sources)?)))
            }
            Some(("file", file)) => {
                values.push((path.clone(), Value::Raw(from_file(&path, Path::new(file))?)))
            }
            Some(("vault", name)) => match sources.vault.handle_for_name(name) {
                Some(handle) => values.push((path, Value::InVault(handle))),
                None => {
                    return Err(secret_error(
                        &path,
                        &format!("names the vault key `{name}`, which is not registered"),
                        "register the key with Builder::on_vault before boot step 8",
                    ));
                }
            },
            Some(_) | None => {
                if !sources.allow_plain && marks_secret(&path) {
                    return Err(Error::new(
                        ErrorKind::PlainSecret,
                        format!(
                            "the configuration key `{path}` holds a plain value, but its name marks a secret"
                        ),
                        "replace the value with env:NAME, file:/path, or vault:name, or set [secrets] allow_plain = true",
                    ));
                }
            }
        }
    }
    Ok(Resolved { values })
}

impl Resolved {
    /// Stores the values in the vault under names for `version` and
    /// returns the table for that configuration. On a failure the values
    /// stored so far are removed again.
    pub(crate) fn install(self, vault: &KeyVault, version: u64) -> Result<Arc<Table>, Error> {
        let mut table = Table {
            vault: vault.clone(),
            handles: HashMap::with_capacity(self.values.len()),
            owned: Vec::new(),
        };
        for (path, value) in self.values {
            let handle = match value {
                Value::InVault(handle) => handle,
                Value::Raw(raw) => {
                    let name = vault_name(version, &path);
                    if let Some(stale) = vault.handle_for_name(&name) {
                        // Left by a reload that failed between its two
                        // phases: never read by any configuration.
                        let _removed = vault.unregister(stale);
                    }
                    match vault.register(name, raw) {
                        Ok(handle) => {
                            table.owned.push(handle);
                            handle
                        }
                        Err(e) => {
                            table.retire();
                            return Err(secret_error(
                                &path,
                                &format!("could not be stored in the vault: {e}"),
                                "check the process's memory limits (RLIMIT_MEMLOCK)",
                            ));
                        }
                    }
                }
            };
            let _previous = table.handles.insert(path, handle);
        }
        Ok(Arc::new(table))
    }
}

/// The vault and the tables of the configuration in force and the one
/// before it (a `Secret` taken from it keeps working through one reload).
pub(crate) struct State {
    vault: KeyVault,
    #[cfg(feature = "env")]
    env: Option<env_lib::Env>,
    tables: Mutex<Tables>,
}

/// The table in force and the one before it.
#[derive(Default)]
struct Tables {
    current: Option<Arc<Table>>,
    previous: Option<Arc<Table>>,
}

impl State {
    pub(crate) fn new(vault: KeyVault, #[cfg(feature = "env")] env: Option<env_lib::Env>) -> State {
        State {
            vault,
            #[cfg(feature = "env")]
            env,
            tables: Mutex::new(Tables::default()),
        }
    }

    pub(crate) fn vault(&self) -> &KeyVault {
        &self.vault
    }

    /// Resolves and stores `config`'s references for `version` and attaches
    /// the table to `config`. Nothing the configuration in force reads
    /// changes.
    pub(crate) fn stage(
        &self,
        config: &Config,
        allow_plain: bool,
        version: u64,
    ) -> Result<Arc<Table>, Error> {
        let sources = Sources {
            vault: &self.vault,
            #[cfg(feature = "env")]
            env: self.env.as_ref(),
            allow_plain,
        };
        let table = resolve(config, &sources)?.install(&self.vault, version)?;
        config.set_secrets(Arc::clone(&table));
        Ok(table)
    }

    /// Makes `table` current; the table two versions back is retired.
    pub(crate) fn accept(&self, table: Arc<Table>) {
        let mut tables = self.tables.lock().unwrap_or_else(PoisonError::into_inner);
        let retired = tables.previous.take();
        tables.previous = tables.current.take();
        tables.current = Some(table);
        drop(tables);
        if let Some(old) = retired {
            old.retire();
        }
    }

    /// Removes every configuration secret (shutdown step 7).
    pub(crate) fn wipe(&self) {
        let (current, previous) = {
            let mut tables = self.tables.lock().unwrap_or_else(PoisonError::into_inner);
            (tables.current.take(), tables.previous.take())
        };
        for table in [current, previous].into_iter().flatten() {
            table.retire();
        }
    }
}

/// `("env", "NAME")` for `"env:NAME"`, and so on.
fn reference(text: &str) -> Option<(&'static str, &str)> {
    ["env", "file", "vault"].into_iter().find_map(|kind| {
        text.strip_prefix(kind)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(|rest| (kind, rest))
    })
}

/// Whether a key's last segment marks it as a secret: the word itself, or
/// a name ending in `_<word>` (`db_password`).
pub(crate) fn marks_secret(path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase();
    SECRET_WORDS
        .iter()
        .any(|word| last == *word || last.ends_with(&format!("_{word}")))
}

/// Every string value with its dotted path. Arrays are walked too; their
/// elements share the array's path.
fn collect(value: &config_lib::Value, path: &mut Vec<String>, out: &mut Vec<(String, String)>) {
    match value {
        config_lib::Value::String(text) => out.push((path.join("."), text.clone())),
        config_lib::Value::Table(table) => {
            for (key, value) in table {
                path.push(key.clone());
                collect(value, path, out);
                let _key = path.pop();
            }
        }
        config_lib::Value::Array(items) => {
            for item in items {
                collect(item, path, out);
            }
        }
        _ => {}
    }
}

fn from_env(path: &str, name: &str, sources: &Sources<'_>) -> Result<RawKey, Error> {
    // The process environment first, then the .env file.
    let value = std::env::var(name).ok().filter(|v| !v.is_empty());
    #[cfg(feature = "env")]
    let value = value.or_else(|| {
        sources
            .env
            .and_then(|env| env.get(name).map(|v| v.into_owned()))
            .filter(|v| !v.is_empty())
    });
    #[cfg(not(feature = "env"))]
    let _no_env = sources;
    match value {
        Some(value) => Ok(RawKey::new(value.into_bytes())),
        None => Err(secret_error(
            path,
            &format!("names the environment variable `{name}`, which is unset or empty"),
            "set the variable in the service's environment or its .env file",
        )),
    }
}

fn from_file(path: &str, file: &Path) -> Result<RawKey, Error> {
    if !file.is_absolute() {
        return Err(secret_error(
            path,
            "names a relative file path",
            "write file:/absolute/path",
        ));
    }
    let opened = fs::File::open(file).map_err(|e| {
        secret_error(
            path,
            &format!(
                "names the file {}, which cannot be opened ({})",
                file.display(),
                e.kind()
            ),
            "check that the file exists and that the service's user can read it",
        )
    })?;
    crate::secure_file::check_secret_open(&opened, file, path)?;
    let mut bytes = Vec::new();
    let read = opened
        .take(MAX_SECRET_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            secret_error(
                path,
                &format!(
                    "names the file {}, which cannot be read ({})",
                    file.display(),
                    e.kind()
                ),
                "check the file",
            )
        });
    let raw = RawKey::new(bytes);
    let read = read?;
    if read as u64 > MAX_SECRET_FILE {
        return Err(secret_error(
            path,
            &format!(
                "names the file {}, which is larger than 64 KiB",
                file.display()
            ),
            "a secret file holds one secret; check the path",
        ));
    }
    Ok(strip_newline(raw))
}

/// One trailing `\n` (or `\r\n`) removed, as editors and `echo` add it.
fn strip_newline(raw: RawKey) -> RawKey {
    let bytes = raw.expose_secret();
    let keep = match bytes {
        [.., b'\r', b'\n'] => bytes.len() - 2,
        [.., b'\n'] => bytes.len() - 1,
        _ => return raw,
    };
    // The original buffer is wiped when `raw` drops here.
    RawKey::new(bytes[..keep].to_vec())
}

fn secret_error(path: &str, detail: &str, action: &'static str) -> Error {
    Error::new(
        ErrorKind::Secret,
        format!("the secret reference at `{path}` {detail}"),
        action,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_kinds() {
        assert_eq!(reference("env:DB_PASS"), Some(("env", "DB_PASS")));
        assert_eq!(reference("file:/run/s"), Some(("file", "/run/s")));
        assert_eq!(reference("vault:db"), Some(("vault", "db")));
        assert_eq!(reference("environment"), None);
        assert_eq!(reference("plain"), None);
    }

    #[test]
    fn test_secret_words() {
        for key in [
            "db.password",
            "api.token",
            "x.db_password",
            "s.client_secret",
            "a.API_KEY",
        ] {
            assert!(marks_secret(key), "{key}");
        }
        for key in [
            "db.host",
            "http.port",
            "log.redact",
            "tokenizer.model",
            "secretary.name",
        ] {
            assert!(!marks_secret(key), "{key}");
        }
    }

    #[test]
    fn test_strip_newline() {
        let stripped = strip_newline(RawKey::new(b"hunter2\r\n".to_vec()));
        assert_eq!(stripped.expose_secret(), b"hunter2");
        let stripped = strip_newline(RawKey::new(b"hunter2\n\n".to_vec()));
        assert_eq!(stripped.expose_secret(), b"hunter2\n");
        let kept = strip_newline(RawKey::new(b"hunter2".to_vec()));
        assert_eq!(kept.expose_secret(), b"hunter2");
    }

    #[test]
    fn test_debug_never_shows_the_value() {
        let vault = key_vault::KeyVaultBuilder::new()
            .normalize_with_blake3(false)
            .build();
        let handle = vault
            .register("t", RawKey::new(b"hunter2".to_vec()))
            .unwrap_or_else(|e| panic!("{e}"));
        let secret = Secret {
            name: "db.password".to_owned(),
            handle,
            vault,
        };
        let text = format!("{secret:?}");
        assert_eq!(text, "Secret(\"db.password\", [redacted])");
        assert_eq!(secret.with(<[u8]>::to_vec).ok(), Some(b"hunter2".to_vec()));
    }
}
