# Formal Audit Scope: ic-stable-roaring v0.1.0 (current v1 implementation)

## Mode

Audit mode — verify correctness of the existing Rust v1 implementation as it exists at
commit `fe455ca`. This is not an architecture comparison and does not cover the abandoned v2
experiment.

## Target components

Core durability layer only:

- `src/bitmap.rs` — stable layout, journal, snapshot, `RoaringBitmap` API
- `src/memory.rs` — stable memory I/O abstractions
- `src/lib.rs` — compile-time constants and layout parameters

Evidence inputs, but not independent formalization targets:

- focused unit/property tests in `src/bitmap.rs`
- the historical serialization fixture under `tests/fixtures/`

Out of scope:

- benchmarks (`src/bench/`)
- fuzz harness
- `MemoryManager` integration (assumed isolated `Memory`)
- `roaring` crate serialization format internals (assumed correct)
- removed or abandoned v2 designs and any nonexistent `src/bitmap_v2.rs`
- performance and benchmark claims

## Properties to verify (priority order)

1. **No silent durable corruption:** every stable image accepted by `init` denotes exactly the
   logical state represented by its validated snapshot and contiguous journal prefix; malformed,
   torn, stale, or contradictory reachable state must not be accepted as a different valid state.
2. **Recoverability of committed operations:** after any successful `insert`, `clear`,
   `ensure_len`, or `truncate`, reopening reconstructs the same logical state.
3. **Checkpoint cutover safety:** at every modeled interruption point in snapshot write, header
   publication, and journal clearing, recovery either yields the pre-checkpoint state, yields the
   checkpoint state, or rejects the image; it must not silently yield a third state.
4. **Error atomicity:** when a public mutation returns an error, its observable heap state and
   recoverable durable state remain equivalent to the pre-call state.
5. **Replay validity:** replay accepts every journal sequence emitted by the public API, rejects
   contradictory/no-op reachable records as intended by Rust, stops at the first empty slot, and
   never applies bytes beyond that prefix.
6. **Representation integrity:** header fields, capacity-derived offsets, 33-bit length encoding,
   `u32` index bounds, 40-bit packing, snapshot bounds, and little-endian memory I/O correspond to
   the Rust representation without idealizing overflow or truncation.
7. **Heap/durable refinement:** while an instance is live, `len` and `contains` match the logical
   state obtained by replaying the durable image plus precisely those writes already committed by
   the modeled operation.

## Assumptions / threat model

- The `Memory` implementation behaves according to the `ic-stable-structures::Memory` contract:
  reads return previously written bytes, writes persist until changed, and `grow` returns `-1` only
  on allocation failure.
- A `RoaringBitmap` instance has exclusive access to its `Memory`; no aliased writes occur.
- The `roaring::RoaringBitmap` (heap crate) serialization/deserialization and set operations are correct.
- Single-writer concurrency only; no concurrent threads access the same instance.
- Memory capacity is finite; `GrowFailed` is possible.
- Lean may treat each individual `Memory::write` call as indivisible, but must model interruption
  between separate writes and between chunks emitted by `serialize_into` / `write_zero_bytes`.
- Whether an IC trap rolls back stable-memory writes is not assumed inside the core proof. When that
  platform guarantee is used to discharge interruption cases, it is isolated as a platform-specific
  commit model and reported separately.

Platform specialization: `Audit.IcMessage` separately models the official ICP rule that a trap or
panic discards current-message state changes. It is not imported into the generic `Memory` model,
and does not weaken the per-write interruption counterexample. The correspondence assumes the
checkpoint runs synchronously inside one message execution without an outgoing-call boundary, as
the audited Rust implementation does.

## Method

Follow the lean-formal-audit skill stages:

1. Define a small logical bitmap plus a decoder-level durable image and explicit write/failure
   trace; add byte-level refinement only for boundaries that cannot otherwise be discharged.
2. Formalize the Rust recovery, journal append, mutation, and checkpoint boundaries with current
   source-line references.
3. Prove refinement and recovery properties in severity order; failed proofs become findings, not
   stronger hidden assumptions.
4. Review every `axiom`, `sorry`, idealization, and source correspondence before reporting.

## Confirmation gate

This scope reflects the user's answers from 2026-07-13 and has been confirmed. Subsequent Lean work
must remain within this v1 audit scope unless the user explicitly revises it.
