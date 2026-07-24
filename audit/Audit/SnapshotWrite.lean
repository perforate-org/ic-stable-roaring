/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Mathlib.Data.List.Basic

/-! # Sequential snapshot-write refinement

This file models only the byte boundary exposed by `MemoryWriter`: serializer chunks are written
without overlap at an advancing offset. It deliberately leaves the Roaring decoder abstract.

Rust references:
- `src/memory.rs` L147-L171: advancing `MemoryWriter` writes
- `src/bitmap.rs` L810-L814: streaming checkpoint serialization
-/

namespace Audit.SnapshotWrite

/-- One durable byte. This preserves the Rust `u8` representation domain rather than treating
arbitrary naturals as bytes. See `src/memory.rs` L159-L168. -/
abbrev Byte : Type := Fin 256

/-- Apply complete serializer chunks at consecutive offsets. After writing one chunk, recursion
continues over the untouched old suffix at exactly the advanced offset. Mirrors
`MemoryWriter::write` in `src/memory.rs` L159-L168. -/
def streamWrites (old : List Byte) : List (List Byte) → List Byte
  | [] => old
  | chunk :: rest => chunk ++ streamWrites (old.drop chunk.length) rest

/-- Sequential non-overlapping writes always produce the concatenated new prefix followed by the
untouched old suffix. This is the byte-level invariant of `MemoryWriter` in
`src/memory.rs` L159-L168. -/
theorem stream_writes_eq_prefix_splice (old : List Byte) (chunks : List (List Byte)) :
    streamWrites old chunks = chunks.flatten ++ old.drop chunks.flatten.length := by
  induction chunks generalizing old with
  | nil => simp [streamWrites]
  | cons chunk rest ih =>
      simp only [streamWrites, List.flatten_cons, ih, List.length_append, List.drop_drop]
      simp [List.append_assoc]

/-- Pre-growth makes the complete serialized snapshot fit in the existing memory region, so the
streaming writes preserve that region's length. Mirrors `grow_memory_to_at_least_bytes` before
serialization in `src/bitmap.rs` L805-L813. -/
theorem stream_writes_length (old : List Byte) (chunks : List (List Byte))
    (h_fit : chunks.flatten.length ≤ old.length) :
    (streamWrites old chunks).length = old.length := by
  rw [stream_writes_eq_prefix_splice, List.length_append, List.length_drop]
  exact Nat.add_sub_of_le h_fit

/-- Bytes visible after the first `k` atomic serializer writes. Interruptions are modeled only
between complete `Memory::write` calls, matching `audit/SCOPE.md` and `src/memory.rs` L165. -/
def bytesAt (old : List Byte) (chunks : List (List Byte)) (k : Nat) : List Byte :=
  streamWrites old (chunks.take k)

/-- Number of bytes written after the first `k` complete serializer chunks. -/
def writtenBytes (chunks : List (List Byte)) (k : Nat) : Nat :=
  (chunks.take k).flatten.length

/-- Absolute exclusive endpoint after the first `k` complete serializer chunks. Mirrors the
advancing `offset` in `MemoryWriter::write` at `src/memory.rs` L159-L168. -/
def writeEnd (base : Nat) (chunks : List (List Byte)) (k : Nat) : Nat :=
  base + writtenBytes chunks k

/-- A prefix of serializer chunks never contains more bytes than the complete serialization. -/
theorem written_bytes_le_total (chunks : List (List Byte)) (k : Nat) :
    writtenBytes chunks k ≤ chunks.flatten.length := by
  induction chunks generalizing k with
  | nil => simp [writtenBytes]
  | cons chunk rest ih =>
      cases k with
      | zero => simp [writtenBytes]
      | succ k =>
          simp only [writtenBytes, List.take_succ_cons, List.flatten_cons, List.length_append]
          exact Nat.add_le_add_left (ih k) chunk.length

/-- Checkpoint pre-growth for the complete encoded length bounds every later serializer write
endpoint. Thus `MemoryWriter`'s repeated grow check cannot request a larger allocation when the
serializer emits exactly that length. Mirrors `src/bitmap.rs` L805-L814. -/
theorem write_end_le_allocated (base allocated : Nat) (chunks : List (List Byte)) (k : Nat)
    (h_allocated : base + chunks.flatten.length ≤ allocated) :
    writeEnd base chunks k ≤ allocated := by
  unfold writeEnd
  exact Nat.le_trans (Nat.add_le_add_left (written_bytes_le_total chunks k) base) h_allocated

/-- At every complete write boundary, the visible bytes have prefix-splice form. This refines the
`snapshotWriting` boundary corresponding to `src/bitmap.rs` L812-L814. -/
theorem bytes_at_eq_prefix_splice (old : List Byte) (chunks : List (List Byte)) (k : Nat) :
    bytesAt old chunks k =
      (chunks.take k).flatten ++ old.drop (chunks.take k).flatten.length := by
  exact stream_writes_eq_prefix_splice old (chunks.take k)

/-- Decoder result at one complete write boundary. The decoder stays abstract because Roaring
codec internals are trusted and out of scope; see `src/bitmap.rs` L524-L533. -/
def observationAt {α : Type} (decode : List Byte → Option α)
    (old : List Byte) (chunks : List (List Byte)) (k : Nat) : Option α :=
  decode (bytesAt old chunks k)

/-- A decoder observation is reachable only when produced after a bounded prefix of actual atomic
serializer writes. This is the refinement relation for `Checkpoint.Stage.snapshotWriting` at
`src/bitmap.rs` L812-L814. -/
def ReachableObservation {α : Type} (decode : List Byte → Option α)
    (old : List Byte) (chunks : List (List Byte)) (observed : Option α) : Prop :=
  chunks.flatten.length ≤ old.length ∧
    ∃ k, k ≤ chunks.length ∧ observationAt decode old chunks k = observed

/-- Every reachable decoder observation has a concrete new-prefix/old-suffix byte witness. The
remaining audit obligation is to classify the abstract decoder on these witnesses, not on arbitrary
observations. Corresponds to interruptions between writes from `src/memory.rs` L165-L167. -/
theorem reachable_observation_has_prefix_splice {α : Type}
    (decode : List Byte → Option α) (old : List Byte)
    (chunks : List (List Byte)) (observed : Option α)
    (h : ReachableObservation decode old chunks observed) :
    chunks.flatten.length ≤ old.length ∧
      ∃ k, k ≤ chunks.length ∧
        decode ((chunks.take k).flatten ++ old.drop (chunks.take k).flatten.length) = observed := by
  rcases h with ⟨h_fit, k, hk, h_observed⟩
  refine ⟨h_fit, k, hk, ?_⟩
  simpa [observationAt, bytes_at_eq_prefix_splice] using h_observed

end Audit.SnapshotWrite
