# Verification Report

## Target and mode

- Target: `ic-stable-roaring` commit `fe455ca` (`v0.1.0`), current v1 implementation
- Mode: implementation audit
- Status: complete

## Scope

The audit covers v1 recovery, journal representation and replay, checkpoint ordering, public
mutation durability, and error atomicity. The abandoned v2 design, roaring codec internals,
MemoryManager aliasing, concurrency, and performance claims are out of scope.

## Method

Lean definitions transcribe the current Rust boundaries, then state recovery and refinement
properties over invalid as well as valid durable inputs. Failed or unavailable proofs are recorded
as findings instead of being discharged with axioms or stronger hidden premises.

## Assumptions and trusted boundaries

- The roaring serializer and decoder are correct for complete serialized values.
- `Memory` reads and writes follow the `ic-stable-structures::Memory` contract.
- Each individual `Memory::write` call is indivisible; interruption may occur between calls.
- One bitmap instance has exclusive access to its memory.
- The optional `Audit.IcMessage` specialization relies on the official ICP rule that a trap or
  panic discards all state modifications from the current message execution. This external
  correspondence is not assumed by the generic `Memory` proofs.
- The durable-image model observes decoded snapshots and raw 40-bit journal slots. Snapshot writes
  additionally use fixed-width bytes to characterize sequential prefix splices. The raw journal
  slots are proved equivalent to their fixed five-byte little-endian representation.
- Charon `0.1.223`, Aeneas revision `d71d2e3`, rustc, and the Lean encoding supplied by Aeneas are
  trusted translation infrastructure. The pinned Aeneas Lean dependency contains four admitted
  generic `Slice`/`StringIter` declarations; the translated journal codec does not use those APIs.

## Findings per file

### `Audit/Journal.lean`

The numeric 40-bit field layout, valid constructor round trips, and malformed tag/reserved/high-bit
rejection are proved.

### `Audit/JournalBytes.lean`

The exact `[u8; 5]` shape is modeled as five `Fin 256` fields. Little-endian encoding and decoding
are proved inverse in both directions, and valid abstract records still decode identically after
the byte round trip.

### `Audit/JournalRefinement.lean`

The production codec was extracted without behavioral changes into `src/journal.rs`, which is also
compiled by a small Aeneas harness rather than copied into verification-only Rust. Lean proves the
generated `set_len` and `set_bit` constructors produce the numeric records defined by
`Audit.Journal`, generated `raw` is exactly the five-byte decoder, and generated `unpack` agrees
with strict model decoding for every possible `[u8; 5]` record.

The checked-in generated files carry a manifest of the Rust and harness Git object hashes. CI
rejects stale input hashes and builds the refinement. Reproducible regeneration uses the pinned
official Aeneas Nix flake.

### `Audit/Bitmap.lean`

Header validation, trusted snapshot decoding, first-empty journal semantics, strict replay, and
accepted-state length bounds are modeled. The input is a decoder-level durable observation, not a
byte array.

### `Audit/Checkpoint.lean`

Successful growth, partial and complete snapshot observations, ordered header publication, and
journal clearing are represented. For the audited default capacity, the entire journal region is
smaller than the 32 KiB zeroing buffer, so checkpoint journal clearing requires at most one durable
write. Completed checkpoint recovery is proved.

The Rust tests deep-copy memory after every actual `Memory::write` and classify each image through
the real `init`. The original array-preserving, array/bitmap-threshold, and run-container cases
recover the current state or reject. The minimized cross-container case below demonstrates that
this property does not hold in general.

### `Audit/SnapshotWrite.lean`

The advancing `MemoryWriter` boundary is refined to `Fin 256` bytes and atomic serializer chunks.
Every complete write boundary is proved equal to the flattened new prefix followed by the untouched
old-memory suffix. The pre-growth fit premise also proves that streaming preserves the allocated
region length. Every prefix endpoint is additionally proved at or below the complete serialized
endpoint, so complete-length pre-growth covers each later `MemoryWriter` call. Reachable decoder
observations must therefore have a bounded prefix-splice witness; the decoder itself remains
abstract under the confirmed trusted-codec boundary.

### `Audit/ErrorAtomicity.lean`

The public mutation boundary distinguishes success from logical-limit, borrow-conflict, checkpoint
address-overflow, and checkpoint pre-growth failures. Each listed returned error occurs before the
attempted mutation's first durable write and is proved to preserve the complete heap and durable
runtime image. Given the trusted premise that the serializer emits its advertised encoded length,
the pre-growth theorem eliminates later per-chunk address/growth errors from `MemoryWriter`.

This result does not treat traps or interruption after a write as returned errors. It also relies on
the documented `Memory` contract and the trusted serializer boundary.

### `Audit/IcMessage.lean`

The generic checkpoint stages, including arbitrary partial-snapshot decoder observations, are
retained unchanged. A separate ICP commit function classifies synchronous checkpoint attempts as
success, returned pre-write error, or trap at a concrete stage. Success commits only the completed
checkpoint image; error and trap commit the pre-message image.

From this platform rule, Lean proves that every committed ICP checkpoint outcome reopens to the
known pre-state or completed target state. The theorem does not constrain generic `Memory`
implementations whose writes persist across process interruption, and therefore does not erase the
counterexample below.

### `Audit/CheckpointAppend.lean`

The successful journal-full path is composed at the durable-image boundary. Given the explicit
canonical-zero journal invariant, writing the first valid record after a completed checkpoint is
proved to succeed, and reopening yields exactly the strict post-record state. Error and
interruption paths remain separate obligations rather than being hidden in this success theorem.

### `Audit/Mutation.lean`

Writing at `journal_len`, decoding a canonical zero tail, and extending replay by one valid record
are proved for the steady-state append path. A concrete Lean witness shows that a valid non-zero
record after the first empty slot becomes visible after the next append.

### `Audit/JournalInvariant.lean`

Crate-generated journals are modeled as a nonzero used prefix followed by an all-zero unused tail.
Fresh creation satisfies the invariant, valid direct append preserves it, and completed checkpoint
clearing returns the full fixed journal to the all-zero state. The checkpoint-append recovery theorem
therefore derives its zero-tail and capacity premises from this invariant for isolated crate-owned
histories.

### `Audit/PublicMutation.lean`

The changing `ensure_len`, set, clear, and truncate paths are proved equal to their strict replay
records. Logical limit errors return no successor, and one-record replay from a checkpointed heap
produces the same post-mutation state.

## Findings

### High — Interrupted checkpoint can recover a third logical state

`RoaringBitmap::checkpoint` overwrites the snapshot in place through multiple `MemoryWriter` calls
before publishing the header and clearing the old journal. `Audit.SnapshotWrite` proves every such
boundary has new-prefix/old-suffix byte form. The Rust regression
`checkpoint_cross_container_splice_recovers_third_state` supplies a reachable prefix for which the
real `roaring 0.11.4` decoder accepts a third state.

The minimized old snapshot has container 0 values `{1, 2}` and container 1 local values `{10, 20}`.
The pending journal clears `1` and inserts container 1 local value `30`, so the intended checkpoint
has container cardinalities one and three. After write boundary 5, both new container descriptions
are durable while the offset table and four-value payload remain old. The decoder reads but ignores
the offsets, then validly partitions the old payload as container 0 `{1}` and container 1
`{2, 10, 20}`. Replaying the old journal clears `1` and inserts `30`, yielding container 1
`{2, 10, 20, 30}`. `init` accepts this bitmap even though the intended state is container 0 `{2}`
and container 1 `{10, 20, 30}`.

Thus an interruption between actual snapshot writes can silently recover a different valid bitmap.
This is a confirmed Rust defect under the audit's per-write interruption model, not merely a missing
decoder theorem. The single-write examples remain useful coverage but do not mitigate the
cross-container counterexample. A generic-memory repair requires changing the checkpoint publication
boundary; a decoder axiom cannot resolve it.

Platform applicability is narrower than the generic `Memory` model. The official ICP execution
contract states that a trap or panic discards all canister-state modifications made by the current
message execution, including stable memory. `checkpoint` is synchronous and makes no inter-canister
call, so its intermediate writes are not committed by a trapping standard IC execution. See
[Properties of Message Executions on ICP, Property 5](https://docs.internetcomputer.org/references/message-execution-properties/)
and [Execution errors](https://docs.internetcomputer.org/references/execution-errors).

Accordingly, this remains High for a `Memory` implementation whose writes can persist across a
process interruption between calls, which is the deliberately conservative core threat model. For
normal IC canister execution it is mitigated by an external platform atomicity guarantee. That
guarantee is not silently imported into the generic Lean model; a later platform-specific theorem
is isolated in `Audit.IcMessage`.

### Resolved (formerly Medium) — Durable little-endian journal representation

`Audit.JournalBytes` now proves a bijection between all `Fin (2^40)` raw values and exactly five
little-endian `Fin 256` bytes. It also composes the byte round trip with valid abstract-record
packing and strict decoding. No additional representation assumption remains at this boundary.

### Resolved under the threat model (formerly Medium) — Canonical unused journal tail

Recovery intentionally stops at the first empty journal slot and does not validate later slots.
`append_record` then overwrites that empty slot, making the following slot reachable. The Lean
theorem `concrete_nonzero_tail_exposure` demonstrates an accepted empty prefix followed by stale
`SetBit(1)` data; appending `SetBit(0)` exposes both records to the next recovery.

`Audit.JournalInvariant` now proves that fresh creation, every valid append, and completed checkpoint
clearing preserve a nonzero used prefix followed by an all-zero unused tail. Consequently, normal
crate-generated histories under the confirmed exclusive-memory threat model discharge the mutation
and checkpoint-append premise without an additional assumption.

The counterexample remains important at the API boundary: an arbitrary externally constructed image
accepted by `init` need not be canonical and can expose stale tail records after mutation. Such
external writes are excluded by the documented memory-isolation contract; widening that threat
model would reopen this finding.

### Resolved under the trusted boundaries — Returned mutation errors are atomic

Logical-limit and borrow-conflict errors precede journaling. A journal-full mutation can also return
from checkpoint address calculation or checkpoint pre-growth before serialization begins. Lean
proves these error transitions preserve both the heap mirror and durable image. It also proves that,
when the trusted serializer emits exactly `serialized_size()` bytes, every serializer chunk endpoint
fits the allocation established before the first chunk.

The Rust regression now exercises checkpoint growth failure for changing `insert`, `ensure_len`,
`clear`, and `truncate` calls and checks both the live heap and reopened durable result. This closes
returned-error atomicity under the stated serializer and `Memory` contracts. The High interruption
finding remains separate and open.

## Unproved spots

There are no `sorry`, `axiom`, or vacuous `True` security claims in the audit's own modules. Open
properties are represented by the findings above rather than admitted Lean propositions. The four
admissions in the external Aeneas support dependency are recorded above as part of the translation
TCB and are not reached by the journal codec.

## Conclusion

Recovery and completed checkpoint behavior have a compiling proof foundation. The highest-priority
remaining work is to repair the confirmed third-state recovery during interrupted in-place snapshot
writes for generic persistently interrupted `Memory`, or explicitly specialize the guarantee to ICP
message rollback rather than changing the streaming layout. The latter specialization now compiles
and proves committed recovery is limited to the pre-state or checkpoint target.
Returned errors before the attempted mutation's first write are now proved atomic, and pre-growth is
proved to cover every serializer chunk under the trusted exact-length premise. The successful
journal-full checkpoint and append path is composed at the durable-image boundary from the preserved
canonical journal invariant. Fixed five-byte journal encoding is proved; broader durable-image byte
refinement remains separate.

For the crate's stated IC deployment target, no checkpoint layout change is recommended solely for
trap atomicity: the platform-specific theorem discharges committed outcomes without adding snapshot
buffering or stable-layout complexity. A generic crash-consistency repair should be pursued only if
persistence across interruption outside ICP message rollback becomes an explicit product contract.
