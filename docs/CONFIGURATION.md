<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>CONFIGURATION</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./BOOT-ORDER.md" title="Boot and shutdown order"><b>BOOT ORDER</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>CONFIGURATION</span>
    </sup>
</div>
<br>

Every key the suite reads, with its type, default, and what it does. Every section is optional and every key has a secure default, so a service runs with no configuration file at all. Sections other than the ones below belong to the application and are read through [`Config`](./API.md#config).

## The file

| | |
|---|---|
| Where | the first that applies: `Builder::config_file` or `config_str`; the file named by `<NAME>_CONFIG` (process environment, then the `.env` file); `<config dir>/<name>.conf`; `./<name>.conf` when the process is not elevated |
| Format | config-lib's `conf`: `[section]` headers, `key = value`, `#` comments, quoted strings, `[a, b]` lists. `.toml` with `config-toml`, `.noml` with `config-noml` (the format follows the extension) |
| Size | at most 1 MiB |
| Owner and mode (Unix) | owned by the service's user or root, not group- or world-writable; checked on the opened file. There is no switch to accept another file |
| Unknown keys | refused in the suite's sections below (a misspelt key cannot silently keep its default); not checked in the application's sections |
| Errors | position only (line and column); a value is never quoted |
| Reload | SIGHUP (Unix), the service manager's parameter-change control (Windows), or `System::reload`: all or nothing |

Durations are an integer number of seconds or a string with a unit: `"500ms"`, `"30s"`, `"5m"`, `"2h"`. Settings given to the `Builder` win over the file.

## `[guard]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `lock` | bool | `true` | take the single-instance lock (`Builder::single_instance` wins) |
| `lock_dir` | path | the runtime directory | where the lock file lives |
| `allow_network_lock` | bool | `false` | accept a lock directory on a network filesystem |
| `umask` | octal string | `"0027"` | the process umask (process-wide mode only) |
| `open_files_target` | integer | `65536` | raise the open-file limit towards this (process-wide mode only) |
| `open_files_minimum` | integer | `1024` | refuse to boot below this hard limit |

Takes effect at restart.

## `[log]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `level` | string | `"info"` | `trace`, `debug`, `info`, `warn`, `error` |
| `filter` | string | none | per-target levels, `"info,db=debug"` |
| `format` | string | `"auto"` | standard error format: `auto` (human on a terminal, JSON otherwise), `human`, `json`, `logfmt` |
| `stderr` | bool | `true` | write to standard error |
| `file` | bool | `true` | write JSON lines to `<logs>/<name>.log` (owner-only) |
| `max_bytes` | integer | none | rotate the file at this size as well |
| `rotation` | string | `"daily"` | `never`, `hourly`, `daily` |
| `max_files` | integer | `14` | rotated files kept (0 to 1000) |
| `redact` | list | `[]` | extra field names whose values are redacted |
| `redact_common` | bool | `true` | redact log-io's list of common secret field names |
| `recent_lines` | integer | `64` | log lines kept in memory for crash reports (0 to 10000) |

Takes effect at restart.

## `[crash]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | install the panic hook (process-wide mode only; `Builder::crash_capture` wins) |
| `max_reports` | integer | `8` | crash reports written per process (0 to 1000) |

## `[metrics]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | create the registry (off: a registry nothing exports) |
| `max_series` | integer | `10000` | the cardinality cap (1 to 10 000 000) |

Takes effect at restart.

## `[kernel]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `shutdown_deadline` | duration | `"25s"` | the whole shutdown; at most one hour (`Builder::shutdown_deadline` wins) |
| `member_reserve` | duration | `"2s"` | kept back from the subsystems for the members; shorter than the deadline |
| `notify` | string | `"auto"` | `auto` sends systemd notifications when `NOTIFY_SOCKET` is set; `off` never |
| `reload_signal` | bool | `true` | act on SIGHUP / parameter change (off: received and ignored) |
| `dump_signal` | bool | `true` | act on SIGUSR1 / control 128 (off: received and ignored) |

The deadlines change on reload; the rest takes effect at restart.

## `[ids]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `worker` | integer | `0` | the Snowflake worker id (0 to 1023); unique per process that shares an id space |
| `epoch` | integer | id-forge's | the Snowflake epoch, in milliseconds since 1970 |

Takes effect at restart.

## `[audit]` (feature `audit`)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | open the chain at boot step 13 |
| `file` | path | `<data dir>/audit/<name>.audit` | the chain file (created owner-only) |
| `hash` | string | `"sha256"` | `sha256`, or `blake3` with `audit-blake3`; must match the hash the chain was written with |
| `sync` | string | `"every"` | `every`: a record is on disk before `record` returns; `flush`: synced every second and at shutdown |
| `repair_tail` | bool | `false` | when the last record was cut short by a crash, keep its bytes in `<file>.cut-<offset>` and continue (off: the boot stops with `Audit`) |

A chain that fails verification always stops the boot (`AuditTampered`); no key turns that off. So does a chain shorter than its anchor (`<state dir>/audit/<name>.anchor`, rewritten after every sync, 0.7): do not delete the anchor to get past it, it is the evidence that records were cut. Takes effect at restart.

## `[secrets]` (feature `secrets`)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `allow_plain` | bool | `false` | accept a plain value under a key whose name marks a secret |

### Secret references

Any value of the form below is a reference, resolved at boot step 8 (and on reload) into the vault and read with `Config::secret`:

| Reference | Resolved from | Refused when |
|---|---|---|
| `env:NAME` | the process environment, then the `.env` file | unset or empty |
| `file:/path` | the file's contents, one trailing newline removed | relative path, missing, over 64 KiB, or (Unix) any group or other permission bit, or another owner |
| `vault:name` | a key registered with `Builder::on_vault` | no such name |

A key whose last segment is, or ends with `_` and, one of `password`, `passwd`, `passphrase`, `secret`, `token`, `api_key`, `apikey`, `private_key`, `credentials`, `client_secret`, `secret_key`, `access_key` must hold a reference (`PlainSecret` otherwise, unless `allow_plain`). Without the `secrets` feature every reference is refused (`Secret`), so a reference is never taken as a literal password.

## Environment variables

| Variable | Meaning |
|---|---|
| `<NAME>_CONFIG` | the configuration file |
| `<NAME>_PATH_SCOPE` | `user`, `global`, or `auto` |
| `<NAME>_<KIND>_DIR` | an absolute directory for `DATA`, `CONFIG`, `CACHE`, `LOGS`, `STATE`, `RUNTIME`, `TEMP`, `DOCUMENTS` |
| `NOTIFY_SOCKET`, `WATCHDOG_USEC`, `WATCHDOG_PID` | set by systemd |

`<NAME>` is the application name in upper case with every other character an underscore (`hive-db` gives `HIVE_DB`). There are no implicit environment overrides of individual keys; a value that must come from the environment says so with an `env:` reference.

## Example

```ini
# /etc/hive/hive.conf (chmod 640)
[log]
level = "info"
redact = ["session_id"]

[kernel]
shutdown_deadline = "40s"

[ids]
worker = 3

[audit]
sync = "every"

[database]
url = "postgres://db.internal/hive"
password = "file:/run/secrets/db_password"
```
