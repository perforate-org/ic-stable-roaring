# Releasing

This guide is for maintainers publishing `ic-stable-roaring`.

## Prepare the release

1. Ensure the worktree contains only intended, committed changes.
2. Update `version` in `Cargo.toml` and add the release notes to `CHANGELOG.md`.
3. If the header, journal encoding, or `JOURNAL_CAP_SLOTS` changed, document the compatibility and
   migration impact. Do not publish a layout-changing release without reopen coverage.

## Validate

Run the normal quality checks:

```sh
cargo fmt --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
scripts/test_layout_matrix.sh
cargo package --list
cargo publish --dry-run
```

When a change affects benchmarks, the default journal capacity, or canbench targets, regenerate
the complete baseline before release:

```sh
canbench --persist
```

Do not use a filtered `--persist` run: it can leave `canbench_results.yml` incomplete. Confirm the
GitHub Actions quality and fuzz-build jobs are green.

## Publish

1. Commit the release preparation.
2. Create and push an annotated `v<version>` tag.
3. Run `cargo publish` from the clean tagged commit.
4. Create the corresponding GitHub Release using the `CHANGELOG.md` entry.
