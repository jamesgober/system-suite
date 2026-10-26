//! Finding, checking, reading, and parsing the configuration file.

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use super::Config;
use crate::{Error, ErrorKind};

/// The largest configuration (or `.env`) file accepted.
pub(crate) const MAX_FILE_BYTES: u64 = 1 << 20;

/// Where the configuration comes from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// `Builder::config_str`.
    Text(String),
    /// A file.
    File(PathBuf),
    /// No file: every section takes its defaults.
    Defaults,
}

/// How the source was found, for the first log lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Builder,
    ProcessEnvironment,
    EnvFile,
    ConfigDirectory,
    WorkingDirectory,
    NotFound,
}

impl Origin {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Origin::Builder => "set by the builder",
            Origin::ProcessEnvironment => "named by the process environment",
            Origin::EnvFile => "named by the .env file",
            Origin::ConfigDirectory => "found in the configuration directory",
            Origin::WorkingDirectory => "found in the working directory",
            Origin::NotFound => "no file found; defaults",
        }
    }
}

/// What finding needs to know.
pub(crate) struct Search<'a> {
    pub(crate) name: &'a str,
    /// The environment-variable prefix, without the trailing `_`.
    pub(crate) prefix: &'a str,
    pub(crate) explicit_file: Option<&'a Path>,
    pub(crate) explicit_text: Option<&'a str>,
    /// `<NAME>_CONFIG` as read from the `.env` file, when it is set there
    /// and not in the process environment.
    pub(crate) env_file_value: Option<String>,
    pub(crate) config_dir: Option<&'a Path>,
    /// Whether the process runs elevated; `None` when unknown.
    pub(crate) elevated: Option<bool>,
}

/// Finds the configuration source (`docs/API.md`, "Finding the file").
pub(crate) fn find(search: &Search<'_>) -> Result<(Source, Origin), Error> {
    if let Some(text) = search.explicit_text {
        return Ok((Source::Text(text.to_owned()), Origin::Builder));
    }
    if let Some(path) = search.explicit_file {
        return Ok((
            Source::File(must_exist(path, "the builder's config_file")?),
            Origin::Builder,
        ));
    }
    let variable = format!("{}_CONFIG", search.prefix);
    if let Some(value) = std::env::var_os(&variable).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        return Ok((
            Source::File(must_exist(&path, &variable)?),
            Origin::ProcessEnvironment,
        ));
    }
    if let Some(value) = search.env_file_value.as_deref().filter(|v| !v.is_empty()) {
        let path = PathBuf::from(value);
        return Ok((Source::File(must_exist(&path, &variable)?), Origin::EnvFile));
    }
    let file_name = format!("{}.conf", search.name);
    if let Some(dir) = search.config_dir {
        let path = dir.join(&file_name);
        if path.is_file() {
            return Ok((Source::File(path), Origin::ConfigDirectory));
        }
    }
    // A working directory is easy to influence: a privileged process (or one
    // whose privilege is unknown) must name its file.
    if search.elevated == Some(false) {
        let path = PathBuf::from(&file_name);
        if path.is_file() {
            return Ok((Source::File(path), Origin::WorkingDirectory));
        }
    }
    Ok((Source::Defaults, Origin::NotFound))
}

fn must_exist(path: &Path, named_by: &str) -> Result<PathBuf, Error> {
    if path.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err(Error::new(
            ErrorKind::ConfigNotFound,
            format!(
                "the configuration file {} ({named_by}) does not exist",
                path.display()
            ),
            "create it, or fix the path",
        ))
    }
}

/// Reads and parses `source` as configuration `version`.
pub(crate) fn load(source: &Source, version: u64) -> Result<Config, Error> {
    let config = match source {
        Source::Text(text) => parse(text, None, version)?,
        Source::File(path) => parse(
            &read_checked(path, "configuration file")?,
            Some(path),
            version,
        )?,
        Source::Defaults => parse("", None, version)?,
    };
    refuse_references(&config)?;
    Ok(config)
}

/// Parses `conf` text.
pub(crate) fn parse(text: &str, source: Option<&Path>, version: u64) -> Result<Config, Error> {
    let document = config_lib::Config::from_string(text, Some("conf")).map_err(|e| {
        // Only the position: the parser's message may quote the text, and
        // the text may hold a secret.
        let at = match &e {
            config_lib::Error::Parse { line, column, .. } => {
                format!(" at line {line}, column {column}")
            }
            _ => String::new(),
        };
        let file = source.map_or_else(
            || "the configuration".to_owned(),
            |p| p.display().to_string(),
        );
        Error::new(
            ErrorKind::ConfigInvalid,
            format!("{file} does not parse{at}"),
            "fix the line named in the message; values with spaces need quotes",
        )
    })?;
    Ok(Config::new(document, source, version))
}

/// Reads a file the process trusts for configuration: refused when another
/// user can change it (Unix; checked on the opened file, D-049), or when it
/// is larger than 1 MiB.
pub(crate) fn read_checked(path: &Path, what: &str) -> Result<String, Error> {
    let file = fs::File::open(path).map_err(|e| io_error(path, what, &e))?;
    crate::secure_file::check_open(&file, path, what)?;
    let mut text = String::new();
    let read = file
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| io_error(path, what, &e))?;
    if read as u64 > MAX_FILE_BYTES {
        return Err(Error::new(
            ErrorKind::ConfigInvalid,
            format!("the {what} {} is larger than 1 MiB", path.display()),
            "shorten it; a configuration file this large is almost certainly a mistake",
        ));
    }
    Ok(text)
}

pub(crate) fn io_error(path: &Path, what: &str, error: &std::io::Error) -> Error {
    let kind = if error.kind() == std::io::ErrorKind::NotFound {
        ErrorKind::ConfigNotFound
    } else {
        ErrorKind::ConfigInvalid
    };
    Error::new(
        kind,
        format!("the {what} {} cannot be read: {error}", path.display()),
        "check that the file exists and that the service's user can read it",
    )
}

/// Secret references (`env:`, `file:`, `vault:`) are resolved through
/// key-vault from 0.5 (D-025, D-031). Until then a reference is refused, so
/// it is never mistaken for a literal.
pub(crate) fn refuse_references(config: &Config) -> Result<(), Error> {
    let root = config.backend().as_value();
    match find_reference(root, &mut Vec::new()) {
        Some(key) => Err(Error::new(
            ErrorKind::Secret,
            format!(
                "the configuration key `{key}` holds a secret reference, which needs the secrets feature (0.5)"
            ),
            "put the value in the file directly for now, or wait for the secrets feature",
        )),
        None => Ok(()),
    }
}

fn find_reference(value: &config_lib::Value, path: &mut Vec<String>) -> Option<String> {
    match value {
        config_lib::Value::String(text)
            if ["env:", "file:", "vault:"]
                .iter()
                .any(|p| text.starts_with(p)) =>
        {
            Some(path.join("."))
        }
        config_lib::Value::Table(table) => table.iter().find_map(|(key, value)| {
            path.push(key.clone());
            let found = find_reference(value, path);
            let _key = path.pop();
            found
        }),
        config_lib::Value::Array(items) => items.iter().find_map(|item| find_reference(item, path)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search<'a>(name: &'a str, dir: Option<&'a Path>) -> Search<'a> {
        Search {
            name,
            prefix: "SYSTEM_SUITE_LOAD_TEST_NEVER_SET",
            explicit_file: None,
            explicit_text: None,
            env_file_value: None,
            config_dir: dir,
            elevated: Some(true),
        }
    }

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("system-suite-load-{label}-{}", std::process::id()));
        drop(fs::remove_dir_all(&dir));
        fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn test_find_prefers_builder_text_then_file() {
        let mut s = search("app", None);
        s.explicit_text = Some("[a]\nb = 1\n");
        assert!(matches!(
            find(&s).ok(),
            Some((Source::Text(_), Origin::Builder))
        ));
        let mut s = search("app", None);
        let missing = Path::new("/definitely/missing/app.conf");
        s.explicit_file = Some(missing);
        assert_eq!(
            find(&s).err().map(|e| e.kind()),
            Some(ErrorKind::ConfigNotFound)
        );
    }

    #[test]
    fn test_find_uses_the_config_directory_then_defaults() {
        let dir = scratch("find");
        let s = search("app", Some(&dir));
        assert!(matches!(
            find(&s).ok(),
            Some((Source::Defaults, Origin::NotFound))
        ));
        fs::write(dir.join("app.conf"), "[a]\nb = 1\n").ok();
        assert!(matches!(
            find(&s).ok(),
            Some((Source::File(_), Origin::ConfigDirectory))
        ));
        drop(fs::remove_dir_all(&dir));
    }

    #[test]
    fn test_find_env_file_value_must_exist() {
        let mut s = search("app", None);
        s.env_file_value = Some("/definitely/missing.conf".to_owned());
        assert_eq!(
            find(&s).err().map(|e| e.kind()),
            Some(ErrorKind::ConfigNotFound)
        );
    }

    #[test]
    fn test_parse_errors_give_position_not_text() {
        let error = parse("[a]\nb = \"hunter2\n", None, 1).err();
        let text = error.as_ref().map(ToString::to_string).unwrap_or_default();
        if !text.is_empty() {
            assert!(!text.contains("hunter2"), "{text}");
            assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::ConfigInvalid));
        }
    }

    #[test]
    fn test_secret_references_are_refused_with_their_key() {
        let config = parse("[db]\npassword = \"env:DB_PASSWORD\"\n", None, 1)
            .unwrap_or_else(|e| panic!("{e}"));
        let error = refuse_references(&config).err();
        assert_eq!(error.as_ref().map(Error::kind), Some(ErrorKind::Secret));
        assert!(
            error
                .map(|e| e.to_string().contains("db.password"))
                .unwrap_or(false)
        );
        let plain =
            parse("[db]\nhost = \"localhost\"\n", None, 1).unwrap_or_else(|e| panic!("{e}"));
        assert!(refuse_references(&plain).is_ok());
    }

    #[test]
    fn test_large_files_are_refused() {
        let dir = scratch("large");
        let path = dir.join("big.conf");
        fs::write(&path, vec![b'#'; 2 << 20]).ok();
        let error = read_checked(&path, "configuration file").err();
        assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::ConfigInvalid));
        drop(fs::remove_dir_all(&dir));
    }

    #[cfg(unix)]
    #[test]
    fn test_group_writable_files_are_refused() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch("modes");
        let path = dir.join("app.conf");
        fs::write(&path, "[a]\nb = 1\n").ok();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).ok();
        assert_eq!(
            crate::secure_file::check(&path, "configuration file")
                .err()
                .map(|e| e.kind()),
            Some(ErrorKind::ConfigInsecure)
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).ok();
        assert!(crate::secure_file::check(&path, "configuration file").is_ok());
        drop(fs::remove_dir_all(&dir));
    }
}
