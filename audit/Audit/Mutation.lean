/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Bitmap
import Mathlib.Data.List.Basic

/-! # Steady-state journal append

This file models the direct-write branch of `append_record`, before journal-full checkpoint
composition. It makes the unused-tail premise explicit: after the runtime journal slot is written,
the following slot becomes reachable to recovery.

Rust references:
- `src/bitmap.rs` L543-L560: first-empty recovery and `journal_len`
- `src/bitmap.rs` L770-L780: direct journal append
- `src/bitmap.rs` L827-L869: replay
-/

namespace Audit.Mutation

/-- Write one raw record at the runtime journal index. Out-of-range indices fail instead of silently
leaving the list unchanged. Mirrors `write_5_bytes` in `src/bitmap.rs` L776-L779. -/
def writeJournalSlot (journal : List Journal.RawRecord) (index : Nat)
    (raw : Journal.RawRecord) : Option (List Journal.RawRecord) :=
  if index < journal.length then some (journal.set index raw) else none

/-- Writing at the prefix length replaces exactly the first empty slot. This is the list-level form
of using `journal_len` as the write index in `src/bitmap.rs` L776-L779. -/
theorem write_after_prefix (rawPrefix tail : List Journal.RawRecord)
    (raw : Journal.RawRecord) :
    writeJournalSlot (rawPrefix ++ 0 :: tail) rawPrefix.length raw =
      some (rawPrefix ++ raw :: tail) := by
  simp [writeJournalSlot, List.set_append_right]

/-- A valid raw prefix followed by an empty slot decodes to exactly the corresponding records.
Mirrors the replay loop and first-empty return in `src/bitmap.rs` L543-L560. -/
theorem decode_prefix_then_empty
    {raws : List Journal.RawRecord} {records : List Abstract.JournalRecord}
    (h : List.Forall₂ (fun raw record ↦ Journal.decode_record raw = some record) raws records)
    (tail : List Journal.RawRecord) :
    Bitmap.decodeJournalPrefix (raws ++ 0 :: tail) = some records := by
  induction h with
  | nil => simp
  | cons h_decode _ ih =>
      have h_nonempty := (Journal.decoded_record_has_valid_tag _ _ h_decode).1
      simp [Bitmap.decodeJournalPrefix, h_nonempty, h_decode, ih]

/-- With another empty slot after the append position, one direct append extends the decoded prefix
by exactly one record. This is the durable premise needed by steady-state public mutations in
`src/bitmap.rs` L645-L767. -/
theorem decode_after_append
    {raws : List Journal.RawRecord} {records : List Abstract.JournalRecord}
    (h : List.Forall₂ (fun raw record ↦ Journal.decode_record raw = some record) raws records)
    (raw : Journal.RawRecord) (record : Abstract.JournalRecord)
    (h_decode : Journal.decode_record raw = some record)
    (tail : List Journal.RawRecord) :
    Bitmap.decodeJournalPrefix (raws ++ raw :: 0 :: tail) =
      some (records ++ [record]) := by
  induction h with
  | nil =>
      have h_nonempty := (Journal.decoded_record_has_valid_tag _ _ h_decode).1
      simp [Bitmap.decodeJournalPrefix, h_nonempty, h_decode]
  | cons h_head _ ih =>
      have h_nonempty := (Journal.decoded_record_has_valid_tag _ _ h_head).1
      simp [Bitmap.decodeJournalPrefix, h_nonempty, h_head, ih]

/-- Replaying an extended prefix is equivalent to replaying the old prefix and then applying the
new record. Mirrors the left-to-right `apply_record` loop in `src/bitmap.rs` L543-L560. -/
theorem replay_after_append
    (initial before after : Abstract.LogicalBitmap)
    (records : List Abstract.JournalRecord) (record : Abstract.JournalRecord)
    (h_prefix : Bitmap.replay initial records = some before)
    (h_record : Bitmap.applyRecord before record = some after) :
    Bitmap.replay initial (records ++ [record]) = some after := by
  unfold Bitmap.replay at h_prefix ⊢
  simp [h_prefix, h_record]

/-- Counterexample at the raw-prefix boundary: recovery accepts a first empty slot without reading
the non-zero tail, but writing that slot exposes the stale record on the next recovery. This is why
public-mutation refinement requires the isolated-memory zero-tail invariant documented in
`src/bitmap.rs` L48-L50. -/
theorem nonzero_tail_becomes_visible
    (freshRaw staleRaw : Journal.RawRecord)
    (fresh stale : Abstract.JournalRecord)
    (tail : List Journal.RawRecord)
    (h_fresh : Journal.decode_record freshRaw = some fresh)
    (h_stale : Journal.decode_record staleRaw = some stale) :
    Bitmap.decodeJournalPrefix (0 :: staleRaw :: 0 :: tail) = some [] ∧
    writeJournalSlot (0 :: staleRaw :: 0 :: tail) 0 freshRaw =
      some (freshRaw :: staleRaw :: 0 :: tail) ∧
    Bitmap.decodeJournalPrefix (freshRaw :: staleRaw :: 0 :: tail) =
      some [fresh, stale] := by
  have h_fresh_nonempty := (Journal.decoded_record_has_valid_tag _ _ h_fresh).1
  have h_stale_nonempty := (Journal.decoded_record_has_valid_tag _ _ h_stale).1
  constructor
  · simp [Bitmap.decodeJournalPrefix]
  constructor
  · simp [writeJournalSlot]
  · simp [Bitmap.decodeJournalPrefix, h_fresh_nonempty, h_stale_nonempty,
      h_fresh, h_stale]

/-- Concrete witness for `nonzero_tail_becomes_visible`: an accepted empty prefix followed by a
stale `SetBit(1)` becomes a two-record prefix after appending `SetBit(0)`. The records are ordinary
values emitted by `JournalRecord::set_bit` in `src/journal.rs` L43-L45. -/
theorem concrete_nonzero_tail_exposure :
    let freshRaw := Journal.raw_set_bit 0 true (by norm_num)
    let staleRaw := Journal.raw_set_bit 1 true (by norm_num)
    Bitmap.decodeJournalPrefix [0, staleRaw, 0] = some [] ∧
    writeJournalSlot [0, staleRaw, 0] 0 freshRaw = some [freshRaw, staleRaw, 0] ∧
    Bitmap.decodeJournalPrefix [freshRaw, staleRaw, 0] =
      some [.set_bit 0 true, .set_bit 1 true] := by
  dsimp only
  exact nonzero_tail_becomes_visible _ _ _ _ []
    (Journal.pack_unpack_set_bit 0 true (by norm_num))
    (Journal.pack_unpack_set_bit 1 true (by norm_num))

end Audit.Mutation
