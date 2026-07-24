# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## 0.1.1

### Changed

- Reduced journal append cost by skipping redundant stable-memory growth checks for the
  preallocated fixed journal region.
- Reduced checkpoint snapshot cost by reusing the pre-grown stable-memory range for streaming
  snapshot writes.
- Reduced reopen overhead by reading the fixed header in one stable-memory access.
- Added benchmark coverage that separates append, checkpoint, snapshot deserialization, and
  journal replay costs.

### Verification

- Added the Lean v1 durability audit and Rust-to-Lean journal codec refinement.
- Added checkpoint interruption regressions and proof-hygiene checks.

## 0.1.0

### Added

- Stable-memory Roaring bitmap with a heap mirror, append-only mutation journal, and checkpointed
  standard-Roaring snapshots.
- Explicit layout validation for magic, version, journal capacity, snapshot bounds, and journal
  record encoding during initialization.
- Configurable compile-time journal capacity, a layout-matrix test suite, and canbench coverage.
- Historical `roaring` 0.11.4 reader-compatibility fixture, stateful persistence property tests,
  and build-checked cargo-fuzz targets.

### Security

- Documented the valid, isolated stable-memory trust boundary: normal recovery stops at the first
  empty journal slot rather than auditing unreachable storage.
