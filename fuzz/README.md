# Fuzzing

This directory is an isolated `cargo-fuzz` workspace. It is not part of the published crate.

## Targets

- `bitmap_operations_reopen` — bounded public API mutations and periodic reopen checks against a
  `BTreeSet` and logical-length oracle.
- `bitmap_init_bytes` — bounded arbitrary stable-memory images; `init` must return without panic.

## Run locally

From this directory, install the pinned toolchain and `cargo-fuzz` once, then list, build, or run a
target:

```sh
rustup toolchain install nightly-2025-08-07 --profile minimal
cargo +nightly-2025-08-07 install cargo-fuzz --version 0.12.0 --locked

cargo +nightly-2025-08-07 fuzz list
cargo +nightly-2025-08-07 fuzz build
cargo +nightly-2025-08-07 fuzz run bitmap_operations_reopen -- -max_total_time=60
```

`corpus/`, `artifacts/`, and `target/` are local-only. Reproduce an artifact with:

```sh
cargo +nightly-2025-08-07 fuzz run <target> artifacts/<target>/<file>
```

GitHub Actions builds both targets but does not run an unbounded fuzzing campaign.
