# system-suite - API Reference

> **Status: 0.1.0 scaffold.** Only `VERSION` exists. The full public API is written here at the `0.2.0` design lock, from the outline in [`../dev/PLAN.md`](../dev/PLAN.md) section 8.

COPYRIGHT © 2026 James Gober.

---

## Table of Contents

- [VERSION](#version)

---

## VERSION

```rust
pub const VERSION: &str
```

The crate version, populated by Cargo at build time.

```rust
println!("system-suite {}", system_suite::VERSION);
```
