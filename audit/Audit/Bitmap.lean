/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Abstract
import Audit.Journal
import Lean.Elab.Tactic.Omega
import Mathlib.Data.List.Basic
import Mathlib.Tactic.NormNum

/-! # V1 durable-image and recovery model

This file models only information that `RoaringBitmap::init` observes:

* the five header fields;
* whether the trusted `roaring` decoder succeeds and consumes exactly the declared bytes;
* the fixed journal region as 40-bit raw slots;
* strict replay up to the first empty slot.

The internals of the `roaring` serialization codec are deliberately absent because they are trusted
by `audit/SCOPE.md`. Journal encoding is not trusted and remains concrete in `Audit.Journal`.

Rust references:
- `src/bitmap.rs` L8-L40: v1 durable layout
- `src/bitmap.rs` L207-L269: header read and validation
- `src/bitmap.rs` L500-L568: `RoaringBitmap::init`
- `src/bitmap.rs` L827-L869: strict journal replay
- `src/lib.rs` L11-L23: journal and logical-length constants
-/

namespace Audit.Bitmap

/-- Default build-time capacity used by this audited build. See `build.rs`. -/
def JOURNAL_CAP_SLOTS : Nat := 4096

/-- One packed journal record occupies five bytes. See `src/bitmap.rs` L67. -/
def JOURNAL_RECORD_SIZE : Nat := 5

/-- The fixed v1 header occupies 64 bytes. See `src/bitmap.rs` L66. -/
def HEADER_SIZE : Nat := 64

/-- The snapshot begins after the fixed journal region. For the audited default capacity this
offset is already eight-byte aligned. See `src/bitmap.rs` L184-L197. -/
def SNAPSHOT_BASE : Nat := HEADER_SIZE + JOURNAL_CAP_SLOTS * JOURNAL_RECORD_SIZE

/-- One-past the largest `u64`, used to preserve checked-address arithmetic in the model. -/
def U64_LIMIT : Nat := 2 ^ 64

/-- Header magic `RSB`. See `src/bitmap.rs` L64. -/
def MAGIC : List Nat := [82, 83, 66]

/-- Current stable-layout version. See `src/bitmap.rs` L65. -/
def VERSION : Nat := 1

/-- Raw header fields read by `read_header`. Invalid values remain representable so recovery can
reject them. See `src/bitmap.rs` L207-L269. -/
structure Header where
  magic : List Nat
  version : Nat
  len_bits : Nat
  journal_slots : Nat
  snapshot_len_bytes : Nat
  deriving Repr, DecidableEq

/-- Result of the trusted `roaring` decoder. `encoded_len` records the number of bytes consumed,
which models the `reader.is_exhausted()` check in `src/bitmap.rs` L527-L532. -/
structure DecodedSnapshot where
  bits : Finset Nat
  encoded_len : Nat
  deriving DecidableEq

/-- Durable input visible to recovery. `snapshot = none` represents decoder failure. Raw journal
slots stay numeric in this recovery structure; `Audit.JournalBytes` separately proves their exact
five-byte little-endian representation. -/
structure DurableImage where
  header : Header
  allocated_bytes : Nat
  snapshot : Option DecodedSnapshot
  journal : List Journal.RawRecord

/-- Header and memory-bound checks performed before snapshot replay. This includes the checked
snapshot-end calculation from `src/bitmap.rs` L504-L521. -/
def validHeader (img : DurableImage) : Prop :=
  img.header.magic = MAGIC ∧
  img.header.version = VERSION ∧
  img.header.journal_slots = JOURNAL_CAP_SLOTS ∧
  img.header.len_bits ≤ Abstract.MAX_LEN ∧
  img.header.snapshot_len_bytes < U64_LIMIT ∧
  SNAPSHOT_BASE + img.header.snapshot_len_bytes < U64_LIMIT ∧
  SNAPSHOT_BASE + img.header.snapshot_len_bytes ≤ img.allocated_bytes ∧
  img.journal.length = JOURNAL_CAP_SLOTS

instance (img : DurableImage) : Decidable (validHeader img) := by
  unfold validHeader
  infer_instance

/-- Every decoded bit lies within the declared logical length. This is the invariant checked after
snapshot decoding in `src/bitmap.rs` L534-L541. -/
def bitsWithin (bits : Finset Nat) (len : Nat) : Prop :=
  ∀ bit ∈ bits, bit < len

instance (bits : Finset Nat) (len : Nat) : Decidable (bitsWithin bits len) := by
  unfold bitsWithin
  infer_instance

/-- Decode the snapshot portion. A declared length of zero means the empty roaring bitmap without
calling the decoder; otherwise decoding must succeed and consume exactly the declared length.
Mirrors `src/bitmap.rs` L524-L533. -/
def decodeSnapshot (img : DurableImage) : Option (Finset Nat) :=
  if img.header.snapshot_len_bytes = 0 then
    some ∅
  else
    match img.snapshot with
    | none => none
    | some snapshot =>
        if snapshot.encoded_len = img.header.snapshot_len_bytes then
          some snapshot.bits
        else none

/-- Decode the contiguous record prefix, stopping at the first all-zero slot. Non-empty malformed
slots reject the image. Mirrors `src/bitmap.rs` L543-L560. -/
def decodeJournalPrefix : List Journal.RawRecord → Option (List Abstract.JournalRecord)
  | [] => some []
  | raw :: rest =>
      if raw.val = 0 then some []
      else
        match Journal.decode_record raw with
        | none => none
        | some record =>
            match decodeJournalPrefix rest with
            | none => none
            | some records => some (record :: records)

/-- Strict replay of a record. Unlike the abstract public operation, recovery rejects no-op records.
Mirrors `apply_record` in `src/bitmap.rs` L827-L869. -/
def applyRecord (state : Abstract.LogicalBitmap)
    (record : Abstract.JournalRecord) : Option Abstract.LogicalBitmap :=
  match record with
  | .set_len new_len =>
      if new_len > Abstract.MAX_LEN ∨ new_len = state.len_bits then none
      else if new_len < state.len_bits then state.truncate new_len
      else state.ensure_len new_len
  | .set_bit index value =>
      if index ≥ Abstract.MAX_LEN then none
      else if value then
        if state.contains index then none else state.set index true
      else
        if index ≥ state.len_bits ∨ ¬state.contains index then none else state.set index false

/-- Replay records from left to right, matching the loop in `src/bitmap.rs` L543-L560. -/
def replay (initial : Abstract.LogicalBitmap)
    (records : List Abstract.JournalRecord) : Option Abstract.LogicalBitmap :=
  records.foldlM applyRecord initial

/-- Recovery from a v1 durable image. Every rejection branch corresponds to a check in
`RoaringBitmap::init` or `apply_record`; no validity proof is hidden in the input structure. -/
def recoverImage (img : DurableImage) : Option Abstract.LogicalBitmap :=
  if h_header : validHeader img then
    match decodeSnapshot img with
    | none => none
    | some bits =>
        if h_bits : bitsWithin bits img.header.len_bits then
          let initial : Abstract.LogicalBitmap := {
            len_bits := img.header.len_bits
            bits := bits
            h_len := h_header.2.2.2.1
            h_bits := h_bits
          }
          match decodeJournalPrefix img.journal with
          | none => none
          | some records => replay initial records
        else none
  else none

namespace DurableImage

/-- Empty image written by `RoaringBitmap::new`. Snapshot decoding is bypassed because the declared
snapshot length is zero. See `src/bitmap.rs` L457-L471. -/
def empty : DurableImage where
  header := {
    magic := MAGIC
    version := VERSION
    len_bits := 0
    journal_slots := JOURNAL_CAP_SLOTS
    snapshot_len_bytes := 0
  }
  allocated_bytes := SNAPSHOT_BASE
  snapshot := none
  journal := List.replicate JOURNAL_CAP_SLOTS 0

end DurableImage

/-- Recovery ignores every slot after the first empty record. -/
@[simp] theorem decodeJournalPrefix_stops_at_empty (rest : List Journal.RawRecord) :
    decodeJournalPrefix (0 :: rest) = some [] := by
  simp [decodeJournalPrefix]

/-- An image accepted by recovery always satisfies the logical length bound. The substantive fact
is carried by the `LogicalBitmap` result, not assumed about arbitrary durable input. -/
theorem recovered_len_bounded (img : DurableImage) (state : Abstract.LogicalBitmap)
    (_h : recoverImage img = some state) : state.len_bits ≤ Abstract.MAX_LEN := by
  exact state.h_len

end Audit.Bitmap
