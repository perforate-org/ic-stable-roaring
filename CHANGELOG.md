# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

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
