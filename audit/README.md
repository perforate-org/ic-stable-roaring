# ic-stable-roaring Lean audit

This directory formalizes the durable v1 behavior of `ic-stable-roaring` in Lean 4.

The audit covers:

- logical bitmap operations and bounds;
- the 40-bit journal record format;
- v1 header, snapshot, and journal recovery;
- strict replay and malformed-record rejection;
- checkpoint streaming, one-write metadata publication, partial-snapshot observations, and
  final-state recovery.

The `roaring` serialization codec itself is trusted and remains outside the formalization. Its
decoded result and exact consumed length form the snapshot boundary in `Audit/Bitmap.lean`.

## Layout

- `SCOPE.md` — confirmed audit scope, assumptions, and property priority
- `Audit/Abstract.lean` — logical bitmap state and public-operation semantics
- `Audit/Journal.lean` — 40-bit numeric record packing, decoding, and rejection
- `Audit/JournalBytes.lean` — fixed five-byte little-endian journal refinement
- `Audit/JournalRefinement.lean` — refinement from Aeneas-generated Rust semantics
- `Audit/Generated/JournalCore/` — checked-in Aeneas translation of `src/journal.rs`
- `Audit/JournalInvariant.lean` — nonzero used prefix and canonical zero-tail preservation
- `Audit/Bitmap.lean` — durable image validation and recovery
- `Audit/Checkpoint.lean` — checkpoint write stages and final recovery
- `Audit/CheckpointAppend.lean` — completed checkpoint and first durable append composition
- `Audit/ErrorAtomicity.lean` — pre-write returned-error state preservation and pre-growth bounds
- `Audit/IcMessage.lean` — ICP message rollback specialization for checkpoint commit atomicity
- `Audit/SnapshotWrite.lean` — sequential snapshot-byte prefix refinement
- `Audit/Mutation.lean` — steady-state journal append and zero-tail refinement
- `Audit/PublicMutation.lean` — public-operation and strict-replay equivalence
- `Audit/Soundness.lean` — accepted-image and journal soundness theorems
- `Audit/Safety.lean` — strict replay rejection and invariant preservation
- `Audit.lean` — top-level build target

## Build

```sh
lake build
```

To build one module while iterating:

```sh
lake build Audit.Journal
lake build Audit.JournalBytes
lake build Audit.JournalRefinement
lake build Audit.JournalInvariant
lake build Audit.Bitmap
lake build Audit.Checkpoint
lake build Audit.CheckpointAppend
lake build Audit.ErrorAtomicity
lake build Audit.IcMessage
lake build Audit.SnapshotWrite
```

## Rust correspondence

The production five-byte codec lives in dependency-free `src/journal.rs`. The translation harness
imports that exact file, Charon lowers it, and Aeneas emits the Lean definitions under
`Audit/Generated/JournalCore/`. `Audit.JournalRefinement` proves that the generated `set_len`,
`set_bit`, `raw`, and `unpack` semantics agree with `Audit.Journal` and `Audit.JournalBytes`.

Install the pinned official Aeneas Nix flake and regenerate with:

```sh
nix profile add github:aeneasverif/aeneas/d71d2e3f2cb763f7faf15ab606ac9cf32da8dead#aeneas
audit/scripts/regenerate-journal-model.sh
audit/scripts/check-journal-model.sh
```

The Aeneas package bundles Charon `0.1.223`. The regeneration script rejects other Aeneas or
Charon versions. CI builds the checked-in generated semantics and verifies that their source
manifest still matches the production Rust file and translation harness.

## Proof hygiene

Security-relevant claims must not be replaced with vacuous `True` propositions. Every remaining
`sorry`, `axiom`, or underspecified premise must be explained in `REPORT.md` once the audit reaches
the reporting stage.

The audit's own Lean modules contain no admitted proposition. The pinned Aeneas library currently
reports four `sorry` declarations in generic `Slice` and `StringIter` support; the journal
translation does not call those APIs, but Aeneas/Charon and their Lean support library remain part
of the explicitly trusted translation boundary.

Byte-level interruption semantics are intentionally deferred until they can refine the actual
`Memory` contract. A whole-write placeholder would incorrectly make allocation and chunked writes
look atomic.

Checkpoint publishes the header, reserved bytes, fixed journal region, and alignment padding in one
bounded metadata write. Partial in-place snapshot serialization remains represented at the
trusted-decoder boundary; see `REPORT.md` for the byte/codec reachability gap.
