# Releasing

This guide is for maintainers publishing `ic-stable-roaring`.

## Prepare the release

1. Ensure the worktree contains only intended, committed changes.
2. Update `version` in `Cargo.toml` and add the release notes to `CHANGELOG.md`.
3. If the header, journal encoding, or `JOURNAL_CAP_SLOTS` changed, document the compatibility and
   migration impact. Do not publish a layout-changing release without reopen coverage.

## Validate

Because this is a library crate, the root `Cargo.lock` is intentionally not committed.
Generate it at the start of the release validation so all subsequent commands use the same
resolved dependency graph:

```sh
cargo generate-lockfile
```

Run the normal quality checks:

```sh
cargo fmt --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
scripts/test_layout_matrix.sh
cargo package --locked --list
cargo publish --dry-run --locked
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
3. Confirm that the `crates-io` GitHub environment requires maintainer approval and is restricted
   to `v*` tags.
4. Confirm that the crate's crates.io Trusted Publisher points to this repository's `Release`
   workflow and the `crates-io` environment.
5. Approve the release workflow when the environment requests approval.
6. Verify the published crate, package checksum, SBOM, and artifact attestation.

For the first release, publish once manually from the clean tagged commit, then configure the
crates.io Trusted Publisher for the `Release` workflow before using this workflow for subsequent
releases. Do not store a long-lived crates.io token in GitHub Actions secrets.

Create the corresponding GitHub Release using the `CHANGELOG.md` entry after the workflow succeeds.
