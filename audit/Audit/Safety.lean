/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Bitmap

/-! # Replay safety

This file proves the local rejection and invariant-preservation properties used by recovery.

Rust references:
- `src/bitmap.rs` L535-L540: snapshot bit bounds
- `src/bitmap.rs` L827-L869: strict journal replay
-/

namespace Audit.Safety

/-- Heap membership has the same length guard as `RoaringBitmap::contains`. -/
theorem contains_consistent (state : Abstract.LogicalBitmap) (index : Nat) :
    state.contains index = (index < state.len_bits ∧ index ∈ state.bits) := by
  simp [Abstract.LogicalBitmap.contains]

/-- Any successful strict replay step preserves the maximum logical length. -/
theorem apply_record_preserves_length_bound
    (before after : Abstract.LogicalBitmap) (record : Abstract.JournalRecord)
    (_h : Bitmap.applyRecord before record = some after) :
    after.len_bits ≤ Abstract.MAX_LEN := by
  exact after.h_len

/-- Recovery rejects a `SetLen` record that repeats the current length. -/
theorem set_len_noop_rejected (state : Abstract.LogicalBitmap) :
    Bitmap.applyRecord state (.set_len state.len_bits) = none := by
  simp [Bitmap.applyRecord]

/-- Recovery rejects a decoded `SetLen` payload above the API's maximum length.
Mirrors `apply_record` in `src/bitmap.rs` L831-L835. -/
theorem oversized_set_len_rejected (state : Abstract.LogicalBitmap) (len : Nat)
    (h : Abstract.MAX_LEN < len) :
    Bitmap.applyRecord state (.set_len len) = none := by
  simp [Bitmap.applyRecord, h]

/-- Recovery rejects setting an already-set bit, including every in-range state reachable from
the public API. -/
theorem duplicate_set_rejected (state : Abstract.LogicalBitmap) (index : Nat)
    (h : state.contains index = true) :
    Bitmap.applyRecord state (.set_bit index true) = none := by
  simp [Bitmap.applyRecord, h]

/-- Recovery rejects clearing a bit that is not currently set. -/
theorem missing_clear_rejected (state : Abstract.LogicalBitmap) (index : Nat)
    (h : state.contains index = false) :
    Bitmap.applyRecord state (.set_bit index false) = none := by
  simp [Bitmap.applyRecord, h]

end Audit.Safety
