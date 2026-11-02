<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>SECURITY</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./CONFIGURATION.md" title="Configuration"><b>CONFIGURATION</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>SECURITY</span>
    </sup>
</div>
<br>

What the suite defends against, how, what each defence costs, and the test that attacks it. Every threat below has a passing attack test in CI on Linux, macOS, and Windows (the Unix-only and Windows-only rows run where they apply).

## Threats and defences

| Id | Threat | Defence | Cost | Attack test |
|---|---|---|---|---|
| T1 | a second copy of the service corrupts shared data | the single-instance lock, taken before any data file is opened | one `open` and one lock at boot | `test_t1_second_copy_is_refused`: two copies; the second exits 75 |
| T2 | secrets in configuration files or logs | references only (`env:`, `file:`, `vault:`), resolved into a page-locked vault; plain values under secret-looking keys refused; secret files checked on the opened file (no group or other bit, owner); log sinks redact common secret keys; errors name keys, never values; a reference inside a list, or under a disabled secrets member, is refused, never read as a literal | resolution at boot and reload only; reading a secret is key-vault's scoped read | the `test_t2_*` tests: env, file, vault resolution; unresolvable references; plain secrets; a loose secret file; reload; `Debug` and errors never show a value |
| T3 | log, audit, lock, or crash files readable by other users | umask set before any file exists; owner-only directories (`0700`, a protected access control list on Windows); log, audit, and crash files `0600` | none at run time; one access-control-list write per directory at creation on Windows | `test_t3_*`: modes on Unix, access control lists on Windows, for the log, lock, audit, and crash files |
| T4 | a half-started process serving traffic | the boot completes before `start` returns; a failing step unwinds everything that started; the application's door is its last subsystem | none | `test_t4_*`: every step that can fail, made to fail with a real trigger; nothing stays started |
| T5 | data loss on a crash | the panic hook flushes the log and the audit chain (bounded) and writes a crash report | flush time at a crash only | `test_t5_t9_*` and `test_t5_a_worker_panic_*`: a panic on the main thread and in a worker; the last log line and the report exist |
| T6 | a stuck subsystem prevents shutdown forever | every shutdown step bounded by the deadline; a step that does not return is abandoned with exit code 79; the next step still runs | one thread per bounded step at shutdown | `test_t6_*`: a `stop` that never returns |
| T7 | an invalid configuration reload takes the service down | validate (the shutdown deadline included), let every subsystem prepare, then publish and commit; otherwise nothing changes. A stop that arrives while subsystems prepare cancels the reload; a panicking commit is caught. The configuration snapshot is handed to the application read-only, so nothing bypasses the validation | the reload's own work, on its own thread (never the signal thread) | `test_t7_*`: a broken file, an unknown key, a refusing subsystem, an unresolvable secret; `test_a_reload_with_a_bad_deadline_is_refused` |
| T8 | signal races during boot | signals hooked at step 7 and acted on at the next step boundary; a stop during boot unwinds and exits 0 | none | the boot driver's boundary test (a stop at every boundary), and `test_t8_sigterm_during_boot_unwinds` (a real SIGTERM while a subsystem starts, Unix) |
| T9 | crash reports leaking secrets | the report holds build and instance info, the panic message and location, and log lines already redacted by the logger | none | `test_t5_t9_*`: a panic with a secret in scope and in a log field; neither reaches the report or the log |

## Further defences

| Defence | Cost | Since |
|---|---|---|
| a configuration or `.env` file another user can change is refused, checked on the opened file (no switch accepts it) | one `fstat` | 0.4 (D-020, D-048, D-049) |
| a configuration parse error names the position only, never the text | none | 0.4 |
| unknown keys in the suite's sections refused, so a typo cannot keep an insecure default | none | 0.4 |
| a working-directory configuration file is ignored when the process is elevated | one check at boot | 0.4 |
| the audit chain is verified at every boot before anything new is written; a cut tail is kept aside, never silently dropped | reading the chain at boot (linear in its size) | 0.5 (D-060) |
| a secret from an older configuration version is never silently switched by a reload | one vault entry per secret per version kept | 0.5 (D-064) |
| the audit chain's tail is anchored outside the file, so a shortened or replaced chain is refused | one small owner-only write per sync, one read at boot | 0.7 (D-072) |
| the `.env` file is read by the suite and checked on the opened file, so the file checked is the file read | none measurable (one `fstat`) | 0.7 (D-073) |
| the configuration and `.env` files must be regular files, checked before opening, so a FIFO cannot block the boot | one `stat` | 0.7 (D-078) |
| an existing audit chain file another user can change is refused before it is resumed | one `fstat` at boot | 0.7 (D-078) |
| a panicking signal handler does not end the signal intake; reloads and dumps run off the signal thread | one short-lived thread per reload or dump | 0.7 (D-075) |
| no secret outlives the shutdown: a reload that finishes after the vault was wiped has its values removed at once | one atomic load per reload | 0.7 (D-077) |

## Known limits

Each is recorded with its reason in the decision log (D-079).

| Limit | Why it stays |
|---|---|
| `.env` values, `env:` secrets among them, stay in env-lib's map in ordinary memory for the life of the process | the same exposure as the process environment, which the suite never changes; a secret that must be wiped belongs in a `file:` or `vault:` reference |
| on Unix, without the `sys` feature or when the user lookup fails, the owner check is skipped and only the mode check (no group or other write bit) applies | reading the user id is an OS call that belongs in sys-lib; the owner-only directories still apply |
| the panic hook the application installs after boot is replaced at shutdown by the one from before boot | the standard library keeps one process-wide hook and cannot tell whose it is; install yours before boot (the suite's hook calls it) |
| members' set-once global slots (the global logger, the global metrics registry) stay installed after a boot that failed after setting them | the members allow setting them once; a failed boot ends the process in every documented use |
| event handlers run on the thread that tears down, outside the shutdown deadline | they are the application's code and must return quickly; bounding each would cost a thread per event |
| on Windows the file checks rely on the directory's access control list, which is set owner-only when the suite creates it | reading a file's access control list needs OS calls the suite does not make itself |

## Reporting a vulnerability

Report privately to the maintainer at `me@jamesgober.com`, not in a public issue. Include the version, the platform, and a way to reproduce it.
