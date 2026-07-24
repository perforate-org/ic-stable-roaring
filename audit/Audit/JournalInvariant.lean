/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Checkpoint
import Audit.Mutation

/-! # Canonical journal history

This file captures the representation invariant maintained by crate-owned journal histories: the
runtime-used prefix contains only nonzero records and every remaining fixed-capacity slot is zero.

Rust references:
- `src/bitmap.rs` L457-L471: fresh journal zeroing
- `src/bitmap.rs` L770-L780: append at `journal_len`
- `src/bitmap.rs` L800-L824: checkpoint clear and runtime reset
-/

namespace Audit.JournalInvariant

/-- A fixed-capacity journal with exactly `used` nonzero prefix records and a completely zero unused
tail. Mirrors the relation between stable slots and runtime `journal_len` in
`src/bitmap.rs` L543-L560 and L770-L780. -/
def CanonicalJournal (journal : List Journal.RawRecord) (used : Nat) : Prop :=
  used ≤ Bitmap.JOURNAL_CAP_SLOTS ∧
    ∃ usedRecords,
      usedRecords.length = used ∧
      (∀ raw ∈ usedRecords, raw.val ≠ 0) ∧
      journal = usedRecords ++ List.replicate (Bitmap.JOURNAL_CAP_SLOTS - used) 0

/-- Every canonical journal occupies exactly the configured fixed region. Mirrors the capacity
metadata checked in `src/bitmap.rs` L504-L521. -/
theorem canonical_length {journal : List Journal.RawRecord} {used : Nat}
    (h : CanonicalJournal journal used) :
    journal.length = Bitmap.JOURNAL_CAP_SLOTS := by
  rcases h with ⟨h_used, usedRecords, h_length, _, rfl⟩
  simp [h_length]
  omega

/-- The all-zero journal written by `RoaringBitmap::new` is canonical at runtime length zero.
Mirrors `src/bitmap.rs` L457-L471. -/
theorem fresh_canonical :
    CanonicalJournal (List.replicate Bitmap.JOURNAL_CAP_SLOTS 0) 0 := by
  refine ⟨by omega, [], rfl, ?_, ?_⟩
  · simp
  · simp

/-- The concrete durable image produced for an empty bitmap carries the canonical fresh journal.
Mirrors `RoaringBitmap::new` in `src/bitmap.rs` L457-L471. -/
theorem empty_image_canonical :
    CanonicalJournal Bitmap.DurableImage.empty.journal 0 := by
  exact fresh_canonical

/-- One successful nonzero write at `journal_len` produces a canonical successor with used length
increased by one. Mirrors `append_record` in `src/bitmap.rs` L770-L780. -/
theorem append_preserves {journal : List Journal.RawRecord} {used : Nat}
    (raw : Journal.RawRecord)
    (h : CanonicalJournal journal used)
    (h_space : used < Bitmap.JOURNAL_CAP_SLOTS)
    (h_raw : raw.val ≠ 0) :
    ∃ updated,
      Mutation.writeJournalSlot journal used raw = some updated ∧
      CanonicalJournal updated (used + 1) := by
  rcases h with ⟨_, usedRecords, h_length, h_nonzero, rfl⟩
  let tail := List.replicate (Bitmap.JOURNAL_CAP_SLOTS - (used + 1))
    (0 : Journal.RawRecord)
  have h_zero_tail :
      List.replicate (Bitmap.JOURNAL_CAP_SLOTS - used) (0 : Journal.RawRecord) = 0 :: tail := by
    rw [show Bitmap.JOURNAL_CAP_SLOTS - used =
      (Bitmap.JOURNAL_CAP_SLOTS - (used + 1)) + 1 by omega]
    simp [tail, List.replicate_succ]
  refine ⟨usedRecords ++ raw :: tail, ?_, ?_⟩
  · rw [h_zero_tail, ← h_length]
    exact Mutation.write_after_prefix usedRecords tail raw
  · refine ⟨by omega, usedRecords ++ [raw], ?_, ?_, ?_⟩
    · simp [h_length]
    · intro item h_item
      simp only [List.mem_append, List.mem_singleton] at h_item
      rcases h_item with h_item | h_item
      · exact h_nonzero item h_item
      · subst item
        exact h_raw
    · simp [tail]

/-- Clearing an already-zero suffix leaves it unchanged. This is the zero-length active-prefix case
of checkpoint clearing in `src/bitmap.rs` L817-L821. -/
@[simp] lemma clear_zero_journal (count : Nat) :
    Checkpoint.clearActiveJournal
      (List.replicate count (0 : Journal.RawRecord)) = List.replicate count 0 := by
  cases count <;> simp [List.replicate_succ, Checkpoint.clearActiveJournal]

/-- Clearing a nonzero prefix followed by zeros produces one all-zero list of the same combined
length. This is the list-level effect of `write_zero_bytes` over the active prefix in
`src/bitmap.rs` L817-L821. -/
lemma clear_prefix_with_zero_tail (usedRecords : List Journal.RawRecord) (zeroCount : Nat)
    (h_nonzero : ∀ raw ∈ usedRecords, raw.val ≠ 0) :
    Checkpoint.clearActiveJournal
      (usedRecords ++ List.replicate zeroCount (0 : Journal.RawRecord)) =
      List.replicate (usedRecords.length + zeroCount) 0 := by
  induction usedRecords with
  | nil => simp
  | cons raw rest ih =>
      have h_raw : raw.val ≠ 0 := h_nonzero raw (by simp)
      have h_rest : ∀ item ∈ rest, item.val ≠ 0 := by
        intro item h_item
        exact h_nonzero item (by simp [h_item])
      simp only [List.cons_append, Checkpoint.clearActiveJournal, h_raw, ↓reduceIte, ih h_rest]
      simp only [List.length_cons]
      rw [show rest.length + 1 + zeroCount = (rest.length + zeroCount) + 1 by omega]
      simp [List.replicate_succ]

/-- A completed checkpoint clears every used record of a canonical journal and therefore produces
the full all-zero journal required by the next append. Mirrors `src/bitmap.rs` L817-L823. -/
theorem clear_canonical {journal : List Journal.RawRecord} {used : Nat}
    (h : CanonicalJournal journal used) :
    Checkpoint.clearActiveJournal journal =
      List.replicate Bitmap.JOURNAL_CAP_SLOTS 0 := by
  rcases h with ⟨h_used, usedRecords, h_length, h_nonzero, rfl⟩
  rw [clear_prefix_with_zero_tail usedRecords
    (Bitmap.JOURNAL_CAP_SLOTS - used) h_nonzero]
  congr
  omega

/-- The durable journal after a completed checkpoint is canonical with runtime used length zero.
Mirrors the clear followed by `journal_len.set(0)` in `src/bitmap.rs` L817-L823. -/
theorem checkpoint_resets_canonical (before : Bitmap.DurableImage)
    (state : Abstract.LogicalBitmap) (encodedLen used : Nat)
    (h : CanonicalJournal before.journal used) :
    CanonicalJournal
      (Checkpoint.finalImage before state encodedLen).journal 0 := by
  rw [show (Checkpoint.finalImage before state encodedLen).journal =
    Checkpoint.clearActiveJournal before.journal by rfl, clear_canonical h]
  exact fresh_canonical

end Audit.JournalInvariant
