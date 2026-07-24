/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Mathlib.Data.Finset.Basic
import Mathlib.Data.Finset.Card
import Mathlib.Data.List.Basic

/-! # Abstract logical state

This file defines the logical bitmap state and the abstract transitions that
correspond to the public API. It is intentionally independent of durable bytes;
the decoder-level durable image is in `Audit/Bitmap.lean`, while byte-level
refinement remains pending.

References (Rust source):
- `src/bitmap.rs` L76-L89: `HeapState`
- `src/bitmap.rs` L91-L97: `remove_suffix_bits`
- `src/bitmap.rs` L627-L663: `ensure_len`
- `src/bitmap.rs` L686-L721: `set`
- `src/bitmap.rs` L733-L768: `truncate`
- `src/bitmap.rs` L827-L869: `apply_record` / replay
-/

namespace Audit.Abstract

/-- Maximum exclusive logical length supported by the API (2^32).
See `src/lib.rs` L21-L23: `JOURNAL_LEN_MAX`. -/
def MAX_LEN : Nat := 2 ^ 32

/-- Logical bitmap state: an exclusive length and a finite set of set bits,
all strictly below the length. Corresponds to `HeapState { len_bits, bitmap }`
in `src/bitmap.rs` L76-L80. -/
structure LogicalBitmap where
  len_bits : Nat
  bits     : Finset Nat
  h_len    : len_bits ≤ MAX_LEN
  h_bits   : ∀ b ∈ bits, b < len_bits

namespace LogicalBitmap

/-- Bit membership test using the current logical length.
Mirrors `RoaringBitmap::contains` in `src/bitmap.rs` L607-L613. -/
def contains (s : LogicalBitmap) (index : Nat) : Bool :=
  index < s.len_bits && index ∈ s.bits

/-- Cardinality of the logical set of bits. -/
def cardinality (s : LogicalBitmap) : Nat := Finset.card s.bits

/-- An empty logical bitmap. Corresponds to `HeapState::new` in `src/bitmap.rs` L83-L89. -/
def empty : LogicalBitmap where
  len_bits := 0
  bits     := ∅
  h_len    := by decide
  h_bits   := by simp

/-- Grow the exclusive logical length to at least `min_len`.
Mirrors `RoaringBitmap::ensure_len` in `src/bitmap.rs` L645-L663.
Returns `none` if the requested length exceeds `MAX_LEN`. -/
def ensure_len (s : LogicalBitmap) (min_len : Nat) : Option LogicalBitmap :=
  if h1 : min_len > MAX_LEN then none
  else if h2 : min_len ≤ s.len_bits then some s
  else some {
    len_bits := min_len,
    bits     := s.bits,
    h_len    := by omega,
    h_bits   := by
      intro b hb
      have := s.h_bits b hb
      omega
  }

/-- Set or clear a single bit. Extends `len_bits` to `index + 1` when setting a bit.
Mirrors `RoaringBitmap::set` in `src/bitmap.rs` L686-L721.
Returns `none` if the required length exceeds `MAX_LEN`. -/
def set (s : LogicalBitmap) (index : Nat) (value : Bool) : Option LogicalBitmap :=
  let need_len := index + 1
  if h1 : need_len > MAX_LEN then none
  else if value then
    some {
      len_bits := max s.len_bits need_len,
      bits     := s.bits ∪ {index},
      h_len    := by
        have h1' : need_len ≤ MAX_LEN := by omega
        have hs : s.len_bits ≤ MAX_LEN := s.h_len
        cases Nat.le_total s.len_bits need_len with
        | inl h => simp [max_eq_right h]; omega
        | inr h => simp [max_eq_left h]; omega,
      h_bits   := by
        intro b hb
        simp only [Finset.mem_union, Finset.mem_singleton] at hb
        rcases hb with (hb | rfl)
        · have := s.h_bits b hb
          cases Nat.le_total s.len_bits need_len with
          | inl h => simp [max_eq_right h]; omega
          | inr h => simp [max_eq_left h]; omega
        · cases Nat.le_total s.len_bits need_len with
          | inl h => simp [max_eq_right h]; omega
          | inr h => simp [max_eq_left h]; omega
    }
  else
    if h2 : index ≥ s.len_bits then some s
    else some {
      len_bits := s.len_bits,
      bits     := s.bits \ {index},
      h_len    := s.h_len,
      h_bits   := by
        intro b hb
        simp only [Finset.mem_sdiff, Finset.mem_singleton] at hb
        have := s.h_bits b hb.1
        omega
    }

/-- Shrink the exclusive logical length to `new_len`, clearing bits ≥ new_len.
Mirrors `RoaringBitmap::truncate` in `src/bitmap.rs` L750-L768. -/
def truncate (s : LogicalBitmap) (new_len : Nat) : Option LogicalBitmap :=
  if h1 : new_len > MAX_LEN then none
  else if h2 : new_len ≥ s.len_bits then some s
  else some {
    len_bits := new_len,
    bits     := s.bits.filter (fun b => b < new_len),
    h_len    := by omega,
    h_bits   := by
      intro b hb
      simp at hb
      omega
  }

end LogicalBitmap

/-- Abstract journal records. These match the on-disk 5-byte records documented
in `src/journal.rs` L22-L28. -/
inductive JournalRecord
  | set_len (len : Nat)
  | set_bit (index : Nat) (value : Bool)
  deriving Repr, BEq

end Audit.Abstract
