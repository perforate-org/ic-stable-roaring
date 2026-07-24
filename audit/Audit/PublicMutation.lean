/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Mutation
import Lean.Elab.Tactic.Omega

/-! # Public mutation refinement

This file proves that every state-changing public bitmap operation has the same logical effect as
its strict journal replay record. It reuses the existing logical operations instead of defining a
second mutation state machine.

Rust references:
- `src/bitmap.rs` L645-L663: `ensure_len`
- `src/bitmap.rs` L686-L721: `set`, `insert`, and `clear`
- `src/bitmap.rs` L750-L767: `truncate`
- `src/bitmap.rs` L827-L869: strict replay
-/

namespace Audit.PublicMutation

/-- A growing `ensure_len` operation and its `SetLen` replay record have exactly the same result.
Mirrors `src/bitmap.rs` L645-L663 and L831-L841. -/
theorem ensure_len_change_matches_replay
    (state : Abstract.LogicalBitmap) (minLen : Nat)
    (h_max : minLen ≤ Abstract.MAX_LEN)
    (h_change : state.len_bits < minLen) :
    Bitmap.applyRecord state (.set_len minLen) = state.ensure_len minLen := by
  have h_not_over : ¬minLen > Abstract.MAX_LEN := by omega
  have h_not_equal : minLen ≠ state.len_bits := by omega
  have h_not_shrink : ¬minLen < state.len_bits := by omega
  simp [Bitmap.applyRecord, h_not_over, h_not_equal, h_not_shrink]

/-- Setting an absent in-range bit and replaying `SetBit(true)` have exactly the same result.
Mirrors `src/bitmap.rs` L686-L705 and L843-L859. -/
theorem set_true_change_matches_replay
    (state : Abstract.LogicalBitmap) (index : Nat)
    (h_index : index < Abstract.MAX_LEN)
    (h_absent : state.contains index = false) :
    Bitmap.applyRecord state (.set_bit index true) = state.set index true := by
  have h_not_over : ¬index ≥ Abstract.MAX_LEN := by omega
  simp [Bitmap.applyRecord, h_not_over, h_absent]

/-- Clearing a present bit and replaying `SetBit(false)` have exactly the same result.
Mirrors `src/bitmap.rs` L710-L720 and L843-L865. -/
theorem set_false_change_matches_replay
    (state : Abstract.LogicalBitmap) (index : Nat)
    (h_present : state.contains index = true) :
    Bitmap.applyRecord state (.set_bit index false) = state.set index false := by
  have h_index_len : index < state.len_bits := by
    have h_present' := h_present
    simp only [Abstract.LogicalBitmap.contains, Bool.and_eq_true,
      decide_eq_true_eq] at h_present'
    exact h_present'.1
  have h_not_over : ¬index ≥ Abstract.MAX_LEN := by
    have := state.h_len
    omega
  simp [Bitmap.applyRecord, h_not_over, h_present, h_index_len]

/-- A shrinking `truncate` operation and its `SetLen` replay record have exactly the same result.
Mirrors `src/bitmap.rs` L750-L767 and L831-L839. -/
theorem truncate_change_matches_replay
    (state : Abstract.LogicalBitmap) (newLen : Nat)
    (h_change : newLen < state.len_bits) :
    Bitmap.applyRecord state (.set_len newLen) = state.truncate newLen := by
  have h_not_over : ¬newLen > Abstract.MAX_LEN := by
    have := state.h_len
    omega
  have h_not_equal : newLen ≠ state.len_bits := by omega
  simp [Bitmap.applyRecord, h_not_over, h_not_equal, h_change]

/-- An oversized `ensure_len` request has no logical successor. Mirrors `src/bitmap.rs`
L645-L651. -/
theorem ensure_len_limit_error
    (state : Abstract.LogicalBitmap) (minLen : Nat)
    (h_over : Abstract.MAX_LEN < minLen) :
    state.ensure_len minLen = none := by
  simp [Abstract.LogicalBitmap.ensure_len, show minLen > Abstract.MAX_LEN by omega]

/-- An index outside the abstract `u32` domain has no logical successor. The Rust public API uses
`u32`, making this branch unreachable there; see `src/bitmap.rs` L686-L693. -/
theorem set_limit_error
    (state : Abstract.LogicalBitmap) (index : Nat) (value : Bool)
    (h_over : Abstract.MAX_LEN ≤ index) :
    state.set index value = none := by
  have h_need : index + 1 > Abstract.MAX_LEN := by omega
  simp [Abstract.LogicalBitmap.set, h_need]

/-- An oversized `truncate` request has no logical successor. Mirrors `src/bitmap.rs`
L750-L756. -/
theorem truncate_limit_error
    (state : Abstract.LogicalBitmap) (newLen : Nat)
    (h_over : Abstract.MAX_LEN < newLen) :
    state.truncate newLen = none := by
  simp [Abstract.LogicalBitmap.truncate, show newLen > Abstract.MAX_LEN by omega]

/-- After checkpoint has made the current heap the snapshot base, replaying one changing record
produces the same post-mutation state. This bridges `src/bitmap.rs` L800-L824 to L827-L869. -/
theorem replay_one_after_checkpoint
    (before after : Abstract.LogicalBitmap) (record : Abstract.JournalRecord)
    (h_record : Bitmap.applyRecord before record = some after) :
    Bitmap.replay before [record] = some after := by
  exact Mutation.replay_after_append before before after [] record
    (by simp [Bitmap.replay]) h_record

end Audit.PublicMutation
