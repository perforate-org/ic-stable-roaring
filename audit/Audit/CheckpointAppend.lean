/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Checkpoint
import Audit.JournalInvariant
import Audit.Mutation

/-! # Journal-full checkpoint and append composition

This file composes the completed checkpoint image with the first durable journal write. The
canonical-zero premise is explicit because checkpoint clears only the active prefix; preserving a
non-zero unreachable tail can expose stale records after the next append.

Rust references:
- `src/bitmap.rs` L770-L780: journal-full checkpoint followed by direct append
- `src/bitmap.rs` L800-L824: completed checkpoint image
-/

namespace Audit.CheckpointAppend

/-- Durable image after a completed checkpoint and one raw write into the new journal's first
slot. Failure is retained from the checked list-level slot write. -/
def appendAfterCheckpoint (before : Bitmap.DurableImage)
    (state : Abstract.LogicalBitmap) (encodedLen : Nat)
    (raw : Journal.RawRecord) : Option Bitmap.DurableImage := do
  let checkpointed := Checkpoint.finalImage before state encodedLen
  let journal ← Mutation.writeJournalSlot checkpointed.journal 0 raw
  pure { checkpointed with journal }

/-- The successful journal-full branch is durable: if checkpoint clearing produces the canonical
zero journal, then appending one valid record and reopening yields exactly the strict post-record
state. This composes the concrete completed checkpoint and slot-write boundaries without a second
mutation state machine. -/
theorem append_after_checkpoint_recovers
    (before : Bitmap.DurableImage)
    (state after : Abstract.LogicalBitmap) (encodedLen : Nat)
    (raw : Journal.RawRecord) (record : Abstract.JournalRecord)
    (h_encoded : 0 < encodedLen)
    (h_address : Bitmap.SNAPSHOT_BASE + encodedLen < Bitmap.U64_LIMIT)
    (h_journal : before.journal.length = Bitmap.JOURNAL_CAP_SLOTS)
    (h_canonical : Checkpoint.clearActiveJournal before.journal =
      List.replicate Bitmap.JOURNAL_CAP_SLOTS 0)
    (h_decode : Journal.decode_record raw = some record)
    (h_apply : Bitmap.applyRecord state record = some after) :
    (appendAfterCheckpoint before state encodedLen raw).bind Bitmap.recoverImage = some after := by
  have h_encoded_limit : encodedLen < Bitmap.U64_LIMIT := by omega
  have h_encoded_ne : encodedLen ≠ 0 := Nat.ne_of_gt h_encoded
  let tail := List.replicate (Bitmap.JOURNAL_CAP_SLOTS - 2) (0 : Journal.RawRecord)
  have h_capacity : Bitmap.JOURNAL_CAP_SLOTS =
      (Bitmap.JOURNAL_CAP_SLOTS - 2) + 2 := by
    norm_num [Bitmap.JOURNAL_CAP_SLOTS]
  have h_canonical' : Checkpoint.clearActiveJournal before.journal = 0 :: 0 :: tail := by
    rw [h_canonical, h_capacity]
    simp [tail, List.replicate_succ]
  have h_write : Mutation.writeJournalSlot
      (Checkpoint.clearActiveJournal before.journal) 0 raw = some (raw :: 0 :: tail) := by
    rw [h_canonical']
    simpa using Mutation.write_after_prefix [] (0 :: tail) raw
  have h_append : appendAfterCheckpoint before state encodedLen raw = some {
      Checkpoint.finalImage before state encodedLen with journal := raw :: 0 :: tail
    } := by
    simp only [appendAfterCheckpoint, Checkpoint.finalImage, Checkpoint.imageAt]
    rw [h_write]
    rfl
  have h_decode_journal : Bitmap.decodeJournalPrefix (raw :: 0 :: tail) = some [record] := by
    simpa using Mutation.decode_after_append
      (raws := []) (records := []) List.Forall₂.nil raw record h_decode tail
  have h_new_length : (raw :: 0 :: tail).length = Bitmap.JOURNAL_CAP_SLOTS := by
    have h_clear_length := Checkpoint.clear_active_journal_length before.journal
    rw [h_canonical'] at h_clear_length
    simpa [h_journal] using h_clear_length
  rw [h_append]
  simp only [Option.bind_some, Bitmap.recoverImage, Bitmap.validHeader,
    Checkpoint.finalImage, Checkpoint.imageAt, Checkpoint.snapshotImage,
    Checkpoint.grownImage, Checkpoint.headerAt, Checkpoint.targetHeader,
    Checkpoint.grownBytes, Checkpoint.targetSnapshot, h_new_length, state.h_len,
    h_encoded_limit, h_address, le_sup_right, Bitmap.decodeSnapshot, h_encoded_ne,
    h_decode_journal, Bitmap.replay, Bitmap.bitsWithin, ↓reduceIte,
    Option.dite_none_right_eq_some, and_true]
  refine ⟨trivial, state.h_bits, ?_⟩
  simpa using h_apply

/-- For crate-generated isolated-memory histories, the canonical journal invariant supplies both
the fixed journal length and all-zero post-checkpoint tail required above. Mirrors the complete
`checkpoint` then append path in `src/bitmap.rs` L770-L824. -/
theorem append_after_checkpoint_recovers_from_canonical
    (before : Bitmap.DurableImage)
    (state after : Abstract.LogicalBitmap) (encodedLen used : Nat)
    (raw : Journal.RawRecord) (record : Abstract.JournalRecord)
    (h_canonical : JournalInvariant.CanonicalJournal before.journal used)
    (h_encoded : 0 < encodedLen)
    (h_address : Bitmap.SNAPSHOT_BASE + encodedLen < Bitmap.U64_LIMIT)
    (h_decode : Journal.decode_record raw = some record)
    (h_apply : Bitmap.applyRecord state record = some after) :
    (appendAfterCheckpoint before state encodedLen raw).bind Bitmap.recoverImage = some after := by
  exact append_after_checkpoint_recovers before state after encodedLen raw record
    h_encoded h_address
    (JournalInvariant.canonical_length h_canonical)
    (JournalInvariant.clear_canonical h_canonical)
    h_decode h_apply

end Audit.CheckpointAppend
