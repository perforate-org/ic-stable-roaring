/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Checkpoint
import Audit.ErrorAtomicity

/-! # ICP message-level checkpoint atomicity

The generic `Memory` model permits persistence between individual writes and has a concrete
third-state checkpoint counterexample. This file adds a separate platform commit layer for normal
ICP message execution: success commits the completed image, while a returned pre-write error or a
trap commits the pre-message image.

External platform references:
- https://docs.internetcomputer.org/references/message-execution-properties/ Property 5:
  trap/panic modifications are not applied
- https://docs.internetcomputer.org/references/execution-errors: current-message state changes are
  rolled back on trap

Rust references:
- `src/bitmap.rs` L770-L824: synchronous append/checkpoint path with no await or outgoing call
- `src/bitmap.rs` L800-L824: complete checkpoint write order
-/

namespace Audit.IcMessage

/-- Terminal result of one synchronous checkpoint attempt. A trapped result retains the concrete
checkpoint stage at which execution stopped, including arbitrary decoder observations from
`snapshotWriting`. -/
inductive CheckpointResult where
  | success
  | returnedError
  | trapped (stage : Checkpoint.Stage)
  deriving DecidableEq

/-- Durable image committed by the ICP message boundary. This is the isolated external platform
rule: a successful synchronous call publishes the completed checkpoint, while returned pre-write
errors and traps publish none of the attempted writes. The generic images from
`Checkpoint.imageAt` remain unchanged and can still contain the proved counterexample. -/
def committedImage (before : Bitmap.DurableImage) (target : Abstract.LogicalBitmap)
    (encodedLen : Nat) : CheckpointResult → Bitmap.DurableImage
  | .success => Checkpoint.finalImage before target encodedLen
  | .returnedError => before
  | .trapped _ => before

/-- A returned checkpoint error is committed as the unchanged pre-message image. The Rust errors
covered here occur before the attempted mutation's first write; see `Audit.ErrorAtomicity`. -/
theorem returned_error_commits_before (before : Bitmap.DurableImage)
    (target : Abstract.LogicalBitmap) (encodedLen : Nat) :
    committedImage before target encodedLen .returnedError = before := by
  rfl

/-- ICP rollback discards every trapped checkpoint stage, not only stages whose bytes would fail to
decode. The stage is intentionally arbitrary so this includes the concrete third-state boundary. -/
theorem trap_discards_checkpoint_stage (before : Bitmap.DurableImage)
    (target : Abstract.LogicalBitmap) (encodedLen : Nat) (stage : Checkpoint.Stage) :
    committedImage before target encodedLen (.trapped stage) = before := by
  rfl

/-- In particular, any decoder result observed during partial snapshot serialization is discarded
when that synchronous ICP message execution traps. -/
theorem trap_discards_snapshot_observation (before : Bitmap.DurableImage)
    (target : Abstract.LogicalBitmap) (encodedLen : Nat)
    (observed : Option Bitmap.DecodedSnapshot) :
    committedImage before target encodedLen
      (.trapped (.snapshotWriting observed)) = before := by
  rfl

/-- Every image committed by one synchronous ICP checkpoint attempt recovers either the known
pre-message state or the completed checkpoint target. This theorem specializes the generic model;
it does not claim that an arbitrary persistently interrupted `Memory` rolls back writes. -/
theorem committed_checkpoint_recovers_pre_or_target
    (before : Bitmap.DurableImage) (pre target : Abstract.LogicalBitmap)
    (encodedLen : Nat) (result : CheckpointResult)
    (h_before : Bitmap.recoverImage before = some pre)
    (h_encoded : 0 < encodedLen)
    (h_address : Bitmap.SNAPSHOT_BASE + encodedLen < Bitmap.U64_LIMIT)
    (h_journal : before.journal.length = Bitmap.JOURNAL_CAP_SLOTS) :
    Bitmap.recoverImage (committedImage before target encodedLen result) = some pre ∨
      Bitmap.recoverImage (committedImage before target encodedLen result) = some target := by
  cases result with
  | success =>
      exact Or.inr (Checkpoint.final_image_recovers before target encodedLen
        h_encoded h_address h_journal)
  | returnedError => exact Or.inl h_before
  | trapped stage => exact Or.inl h_before

end Audit.IcMessage
