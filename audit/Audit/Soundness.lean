/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Bitmap

/-! # Recovery soundness

These proofs connect packed journal records and accepted durable images to the v1 recovery model.
They intentionally avoid restating definitions as `True`.

Rust references:
- `src/journal.rs` L33-L95: journal pack/unpack
- `src/bitmap.rs` L500-L568: recovery validation and replay
-/

namespace Audit.Soundness

/-- Every record emitted by the packer is accepted by the corresponding unpacker. -/
theorem journal_record_roundtrip
    (record : Abstract.JournalRecord) (raw : Journal.RawRecord)
    (h : Journal.pack_record record = some raw) :
    Journal.decode_record raw = some record := by
  exact Journal.pack_unpack_inverse record raw h

/-- Successful recovery implies that the raw header, allocation bounds, and fixed journal size
passed all v1 validation checks. -/
theorem recovery_requires_valid_header
    (img : Bitmap.DurableImage) (state : Abstract.LogicalBitmap)
    (h : Bitmap.recoverImage img = some state) : Bitmap.validHeader img := by
  unfold Bitmap.recoverImage at h
  split at h
  · assumption
  · contradiction

/-- A recovered state cannot exceed the Rust API's exclusive-length limit. -/
theorem recovery_respects_length_limit
    (img : Bitmap.DurableImage) (state : Abstract.LogicalBitmap)
    (h : Bitmap.recoverImage img = some state) :
    state.len_bits ≤ Abstract.MAX_LEN := by
  exact Bitmap.recovered_len_bounded img state h

/-- Once an empty slot is reached, arbitrary later raw slots cannot affect recovery. -/
theorem journal_tail_after_empty_is_unreachable
    (tail₁ tail₂ : List Journal.RawRecord) :
    Bitmap.decodeJournalPrefix (0 :: tail₁) =
      Bitmap.decodeJournalPrefix (0 :: tail₂) := by
  simp

end Audit.Soundness
