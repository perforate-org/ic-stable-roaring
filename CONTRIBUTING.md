# Contributing

Thank you for contributing to `ic-stable-roaring`. Issues, documentation improvements, tests, and
focused pull requests are all welcome.

## Before you contribute

- Search existing [issues](https://github.com/perforate-org/ic-stable-roaring/issues) before
  opening a new one. Bug reports should include the crate version, a minimal reproduction, and the
  expected and observed behavior.
- Keep each pull request focused on one fix or improvement.
- Do not disclose vulnerabilities in a public issue; follow the private reporting instructions in
  [SECURITY.md](./SECURITY.md).

## Development checks

Run these checks before opening a pull request:

```sh
cargo fmt --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
scripts/test_layout_matrix.sh
audit/scripts/check-journal-model.sh
(cd audit && lake build)
! rg -n '\b(sorry|axiom)\b|UNDERSPECIFIED|True := by|: True' audit --glob '*.lean'
```

The [`fuzz/`](./fuzz/README.md) workspace contains longer-running, manual checks and reproduction
instructions. The Lean audit uses the pinned toolchain in [`audit/lean-toolchain`](./audit/lean-toolchain)
and must remain free of proof placeholders and vacuous security claims.
Changes to `src/journal.rs` must also regenerate the pinned Aeneas model as documented in
[`audit/README.md`](./audit/README.md).

## Stable-memory changes

The header, journal encoding, and `JOURNAL_CAP_SLOTS` define the on-disk layout. Changes to any of
them must document their compatibility and migration impact, and add coverage for initialization or
reopen behavior. A compile-time capacity change is not compatible with existing stable memory.

If a change affects canbench targets or their behavior, regenerate the complete baseline with
`canbench --persist`; do not use a filtered persist run, which can leave the result file incomplete.
