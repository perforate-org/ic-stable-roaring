/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Bitmap
import Audit.SnapshotWrite

/-! # Returned-error atomicity

This file separates errors returned before a durable write from interruption between writes.

Rust references:
- `src/bitmap.rs` L645-L657, L686-L697, L750-L761: logical limits and pre-write checks
- `src/bitmap.rs` L783-L788: borrow-conflict check before journaling
- `src/bitmap.rs` L800-L813: checkpoint address calculation and pre-growth
- `src/memory.rs` L159-L168: bounded advancing serializer writes
-/

namespace Audit.ErrorAtomicity

/-- Heap and durable state jointly observable at a public mutation boundary. -/
structure RuntimeImage where
  heap : Abstract.LogicalBitmap
  durable : Bitmap.DurableImage

/-- Concrete classes of errors that the current implementation returns before the attempted
mutation's first write. -/
inductive ReturnedError where
  | limitsExceeded
  | borrowConflict
  | checkpointAddressOverflow
  | checkpointGrowFailed
  deriving DecidableEq

/-- Result of one public mutation attempt. -/
inductive MutationResult where
  | success
  | error (reason : ReturnedError)
  deriving DecidableEq

/-- Public-mutation boundary relevant to returned-error atomicity. Successful execution may reach
a new runtime image. Each listed error is emitted before checkpoint serialization or journal append,
so its transition retains the caller-visible image. -/
inductive MutationStep : RuntimeImage → MutationResult → RuntimeImage → Prop where
  | success (before after : RuntimeImage) : MutationStep before .success after
  | limitsExceeded (before : RuntimeImage) :
      MutationStep before (.error .limitsExceeded) before
  | borrowConflict (before : RuntimeImage) :
      MutationStep before (.error .borrowConflict) before
  | checkpointAddressOverflow (before : RuntimeImage) :
      MutationStep before (.error .checkpointAddressOverflow) before
  | checkpointGrowFailed (before : RuntimeImage) :
      MutationStep before (.error .checkpointGrowFailed) before

/-- Every modeled returned error preserves both the heap mirror and durable image. This theorem
does not cover traps or interruption after any `Memory::write`. -/
theorem returned_error_preserves_runtime {before after : RuntimeImage} {reason : ReturnedError}
    (h : MutationStep before (.error reason) after) :
    after = before := by
  cases h <;> rfl

theorem returned_error_preserves_heap {before after : RuntimeImage} {reason : ReturnedError}
    (h : MutationStep before (.error reason) after) :
    after.heap = before.heap := by
  rw [returned_error_preserves_runtime h]

theorem returned_error_preserves_durable {before after : RuntimeImage} {reason : ReturnedError}
    (h : MutationStep before (.error reason) after) :
    after.durable = before.durable := by
  rw [returned_error_preserves_runtime h]

/-- If the trusted serializer emits exactly its advertised encoded length and checkpoint has grown
through `base + encoded`, every complete chunk endpoint fits the allocation. This is the reason the
per-chunk `MemoryWriter` grow checks introduce no later returned growth failure. -/
theorem pregrown_serializer_write_fits (base encoded allocated : Nat)
    (chunks : List (List SnapshotWrite.Byte)) (k : Nat)
    (h_encoded : chunks.flatten.length = encoded)
    (h_allocated : base + encoded ≤ allocated) :
    SnapshotWrite.writeEnd base chunks k ≤ allocated := by
  apply SnapshotWrite.write_end_le_allocated
  simpa [h_encoded] using h_allocated

end Audit.ErrorAtomicity
