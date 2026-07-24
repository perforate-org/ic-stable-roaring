/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Bitmap
import Audit.SnapshotWrite
import Mathlib.Data.List.Basic

/-! # Checkpoint write ordering

This file models the durable boundaries visible without formalizing the trusted roaring codec.
Partial snapshot writes are represented by their decoder observation. `Audit.SnapshotWrite`
separately proves that actual sequential write boundaries have new-prefix/old-suffix byte form;
classifying the trusted decoder on those reachable splices remains open.

Rust references:
- `src/bitmap.rs` L800-L824: checkpoint order
- `src/bitmap.rs` L236-L242: header field write order
- `src/memory.rs` L61-L80: chunked journal zeroing
-/

namespace Audit.Checkpoint

/-- `write_zero_bytes` uses one 32 KiB buffer. See `src/memory.rs` L8 and L61-L80. -/
def ZERO_WRITE_CHUNK_BYTES : Nat := 32 * 1024

/-- Observable durable boundaries in checkpoint order. `before` also represents failed pre-growth,
which leaves memory unchanged. `snapshotWriting observed` records what the trusted decoder would
observe after an interrupted in-place serialization write. Reachable observations are constrained
by `SnapshotWrite.ReachableObservation`; this stage remains unconstrained here so checkpoint logic
does not depend on codec bytes. The final runtime `journal_len := 0` has no additional durable
effect. See `src/bitmap.rs` L800-L823. -/
inductive Stage
  | before
  | grown
  | snapshotWriting (observed : Option Bitmap.DecodedSnapshot)
  | snapshotWritten
  | headerWritten
  | journalCleared
  deriving DecidableEq

/-- Decoded result of the trusted serialization write for the target logical state. Mirrors
`src/bitmap.rs` L810-L814 at the trusted-codec boundary. -/
def targetSnapshot (state : Abstract.LogicalBitmap) (encodedLen : Nat) :
    Bitmap.DecodedSnapshot := {
  bits := state.bits
  encoded_len := encodedLen
}

/-- Zero the active prefix up to the first empty slot and preserve the unreachable tail. This is the
effect of clearing `journal_len * 5` bytes in `src/bitmap.rs` L817-L821. -/
def clearActiveJournal : List Journal.RawRecord → List Journal.RawRecord
  | [] => []
  | raw :: rest =>
      if raw.val = 0 then raw :: rest
      else 0 :: clearActiveJournal rest

/-- Maximum allocation required before snapshot serialization begins. See `src/bitmap.rs`
L805-L808. -/
def grownBytes (before : Bitmap.DurableImage) (encodedLen : Nat) : Nat :=
  max before.allocated_bytes (Bitmap.SNAPSHOT_BASE + encodedLen)

/-- Image after successful pre-growth and before any snapshot byte is overwritten. Mirrors
`src/bitmap.rs` L805-L810. -/
def grownImage (before : Bitmap.DurableImage) (encodedLen : Nat) : Bitmap.DurableImage := {
  before with allocated_bytes := grownBytes before encodedLen
}

/-- Canonical header that checkpoint intends to publish. Mirrors `src/bitmap.rs` L801-L816. -/
def targetHeader (state : Abstract.LogicalBitmap) (encodedLen : Nat) : Bitmap.Header := {
  magic := Bitmap.MAGIC
  version := Bitmap.VERSION
  len_bits := state.len_bits
  journal_slots := Bitmap.JOURNAL_CAP_SLOTS
  snapshot_len_bytes := encodedLen
}

/-- Image shared by every stage after snapshot serialization completes. Mirrors
`src/bitmap.rs` L810-L814. -/
def snapshotImage (before : Bitmap.DurableImage) (state : Abstract.LogicalBitmap)
    (encodedLen : Nat) : Bitmap.DurableImage := {
  grownImage before encodedLen with
  snapshot := some (targetSnapshot state encodedLen)
}

/-- Header visible at each publication boundary. Keeping publication in one transition function
avoids a chain of nearly identical whole-image updates. Mirrors `src/bitmap.rs` L236-L242. -/
def headerAt (before : Bitmap.Header) (state : Abstract.LogicalBitmap)
    (encodedLen : Nat) : Stage → Bitmap.Header
  | .before | .grown | .snapshotWriting _ | .snapshotWritten => before
  | .headerWritten | .journalCleared => targetHeader state encodedLen

/-- Durable image at one checkpoint boundary. The header cases mirror the five separate writes in
`Header::write`; journal clearing is one atomic write for the audited default capacity. -/
def imageAt (before : Bitmap.DurableImage) (state : Abstract.LogicalBitmap)
    (encodedLen : Nat) (stage : Stage) : Bitmap.DurableImage :=
  match stage with
  | .before => before
  | .grown => grownImage before encodedLen
  | .snapshotWriting observed => {
      grownImage before encodedLen with snapshot := observed
    }
  | .journalCleared => {
      snapshotImage before state encodedLen with
      header := headerAt before.header state encodedLen stage
      journal := clearActiveJournal before.journal
    }
  | _ => {
      snapshotImage before state encodedLen with
      header := headerAt before.header state encodedLen stage
    }

/-- Final durable checkpoint image after the active journal has been cleared. Mirrors
`src/bitmap.rs` L816-L823. -/
def finalImage (before : Bitmap.DurableImage) (state : Abstract.LogicalBitmap)
    (encodedLen : Nat) : Bitmap.DurableImage :=
  imageAt before state encodedLen .journalCleared

/-- Header publication is complete before journal clearing starts. Mirrors `src/bitmap.rs`
L816-L821. -/
theorem header_at_journal_clear (before : Bitmap.DurableImage)
    (state : Abstract.LogicalBitmap) (encodedLen : Nat) :
    (imageAt before state encodedLen .journalCleared).header =
      targetHeader state encodedLen := by
  rfl

/-- The default journal region fits in one `write_zero_bytes` chunk. Thus checkpoint exposes no
durable state with only part of an active journal slot cleared in this audited build. -/
theorem default_journal_clear_fits_one_write :
    Bitmap.JOURNAL_CAP_SLOTS * Bitmap.JOURNAL_RECORD_SIZE ≤ ZERO_WRITE_CHUNK_BYTES := by
  norm_num [Bitmap.JOURNAL_CAP_SLOTS, Bitmap.JOURNAL_RECORD_SIZE, ZERO_WRITE_CHUNK_BYTES]

/-- Clearing the active prefix preserves the fixed journal region length. Models
`src/bitmap.rs` L817-L821. -/
@[simp] theorem clear_active_journal_length (journal : List Journal.RawRecord) :
    (clearActiveJournal journal).length = journal.length := by
  induction journal with
  | nil => rfl
  | cons raw rest ih =>
      simp only [clearActiveJournal]
      split <;> simp_all

/-- A non-empty journal has an empty first slot after its active prefix is cleared, so recovery
stops immediately as in `src/bitmap.rs` L549-L557. -/
theorem decode_cleared_active_journal (raw : Journal.RawRecord)
    (rest : List Journal.RawRecord) :
    Bitmap.decodeJournalPrefix (clearActiveJournal (raw :: rest)) = some [] := by
  simp only [clearActiveJournal]
  split <;> simp [Bitmap.decodeJournalPrefix, *]

/-- A completed checkpoint reopens to the target logical state. The hypotheses are exactly the
remaining concrete bounds: roaring serialization is non-empty, the snapshot end fits in `u64`, and
the source image owns the fixed-capacity journal region. Mirrors `src/bitmap.rs` L800-L824. -/
theorem final_image_recovers (before : Bitmap.DurableImage)
    (state : Abstract.LogicalBitmap) (encodedLen : Nat)
    (h_encoded : 0 < encodedLen)
    (h_address : Bitmap.SNAPSHOT_BASE + encodedLen < Bitmap.U64_LIMIT)
    (h_journal : before.journal.length = Bitmap.JOURNAL_CAP_SLOTS) :
    Bitmap.recoverImage (finalImage before state encodedLen) = some state := by
  have h_encoded_limit : encodedLen < Bitmap.U64_LIMIT := by omega
  have h_encoded_ne : encodedLen ≠ 0 := Nat.ne_of_gt h_encoded
  have h_decode_empty :
      Bitmap.decodeJournalPrefix (clearActiveJournal before.journal) = some [] := by
    cases h_before : before.journal with
    | nil => simp [h_before, Bitmap.JOURNAL_CAP_SLOTS] at h_journal
    | cons raw rest => exact decode_cleared_active_journal raw rest
  simp only [Bitmap.recoverImage, Bitmap.validHeader, finalImage, imageAt,
    snapshotImage, grownImage, headerAt, targetHeader, grownBytes, targetSnapshot, h_journal,
    clear_active_journal_length, state.h_len, h_encoded_limit,
    h_address, le_sup_right,
    Bitmap.decodeSnapshot, h_encoded_ne, ↓reduceIte, h_decode_empty, Bitmap.replay,
    List.foldlM_nil, Option.pure_def, Option.dite_none_right_eq_some, exists_prop, and_true,
    Bitmap.bitsWithin]
  exact ⟨trivial, state.h_bits⟩

end Audit.Checkpoint
