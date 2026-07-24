/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Abstract
import Lean.Elab.Tactic.Omega
import Mathlib.Data.Nat.Basic
import Mathlib.Tactic.NormNum

/-! # Journal record formalization

This file formalizes the numeric field layout of one journal record. The raw
record is a 40-bit value modeled as `Fin (2^40)`. `Audit.JournalBytes` separately
proves its bijection with the durable little-endian `[u8; 5]` representation.

References (Rust source):
- `src/journal.rs` L6-L95: constants, `JournalTag`, and `JournalRecord`
- `src/journal.rs` L22-L30: on-disk bit layout
- `src/lib.rs` L17-L18: `JOURNAL_RECORD_RAW_MASK`
-/

namespace Audit.Journal

/-- Raw journal record as a 40-bit value. See `src/lib.rs` L18
(`JOURNAL_RECORD_RAW_MASK = (1u64 << 40) - 1`). -/
abbrev RawRecord : Type := Fin (2 ^ 40)

/-- Tag bits (bits 38..40). Mirrors `JournalRecord::unpack` in `src/journal.rs` L76-L80. -/
def tag_bits (raw : RawRecord) : Nat :=
  (raw.val / 2 ^ 38) % 4

/-- Reserved bits (bits 33..37). Mirrors `JournalRecord::unpack` in `src/journal.rs` L73-L75. -/
def reserved_bits (raw : RawRecord) : Nat :=
  (raw.val / 2 ^ 33) % 16

/-- Value bit (bit 37). Mirrors `JournalRecord::unpack` in `src/journal.rs` L81. -/
def value_bit (raw : RawRecord) : Bool :=
  (raw.val / 2 ^ 37) % 2 = 1

/-- Low 32 bits of payload. Mirrors `JournalRecord::unpack` in `src/journal.rs` L83. -/
def payload_lo (raw : RawRecord) : Nat :=
  raw.val % 2 ^ 32

/-- High length bit for `SetLen` records (bit 32).
Mirrors `JournalRecord::unpack` in `src/journal.rs` L82. -/
def len_hi (raw : RawRecord) : Nat :=
  (raw.val / 2 ^ 32) % 2

section PackRaw

/-- Raw `SetLen` encoding. Requires `len ≤ Abstract.MAX_LEN`.
Mirrors `JournalRecord::set_len` / `pack_fields` in `src/journal.rs` L33-L53. -/
def raw_set_len (len : Nat) (h : len ≤ Abstract.MAX_LEN) : RawRecord :=
  ⟨len + 2 ^ 38, by simp [Abstract.MAX_LEN] at h; omega⟩

/-- Raw `SetBit` encoding. Requires `index < 2^32`.
Mirrors `JournalRecord::set_bit` / `pack_fields` in `src/journal.rs` L43-L53. -/
def raw_set_bit (index : Nat) (value : Bool) (h : index < 2 ^ 32) : RawRecord :=
  match value with
  | false => ⟨index + 2 ^ 39, by omega⟩
  | true => ⟨index + 2 ^ 37 + 2 ^ 39, by omega⟩

end PackRaw

/-- Pack an abstract journal record into a raw 40-bit value. Mirrors the
correctness of `JournalRecord::pack_fields` in `src/journal.rs` L47-L53. -/
def pack_record (r : Abstract.JournalRecord) : Option RawRecord :=
  match r with
  | Abstract.JournalRecord.set_len len =>
      if h : len ≤ Abstract.MAX_LEN then some (raw_set_len len h) else none
  | Abstract.JournalRecord.set_bit index value =>
      if h : index < 2 ^ 32 then some (raw_set_bit index value h) else none

/-- Decode one non-empty raw slot into an abstract record. `none` means either that the caller
passed the empty-slot sentinel or that the record is malformed. Recovery distinguishes those cases
before calling this function; see `src/bitmap.rs` L549-L558. For `SetLen`, the value bit is
deliberately ignored, matching `JournalRecord::unpack` in `src/journal.rs` L68-L95. -/
def decode_record (raw : RawRecord) : Option Abstract.JournalRecord :=
  if raw.val = 0 then none
  else if reserved_bits raw ≠ 0 then none -- see `src/journal.rs` L73-L75.
  else
    match tag_bits raw with
    | 1 =>
        let len := len_hi raw * 2 ^ 32 + payload_lo raw
        some (Abstract.JournalRecord.set_len len)
    | 2 =>
        if len_hi raw ≠ 0 then none -- see `src/journal.rs` L86-L89.
        else
          some (Abstract.JournalRecord.set_bit (payload_lo raw) (value_bit raw))
    | _ => none

section PackHelpers

/-- The raw value of a packed `SetLen` record. -/
lemma raw_set_len_val (len : Nat) (h : len ≤ Abstract.MAX_LEN) :
    (raw_set_len len h).val = len + 2 ^ 38 := by
  simp [raw_set_len]

/-- A packed `SetLen` record is never zero. -/
lemma raw_set_len_ne_zero (len : Nat) (h : len ≤ Abstract.MAX_LEN) :
    (raw_set_len len h).val ≠ 0 := by
  rw [raw_set_len_val]
  omega

/-- The raw value of a packed `SetBit` record. -/
lemma raw_set_bit_val (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    (raw_set_bit index value h).val = index + (if value then 2 ^ 37 else 0) + 2 ^ 39 := by
  cases value <;> simp [raw_set_bit]

/-- A packed `SetBit` record is never zero. -/
lemma raw_set_bit_ne_zero (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    (raw_set_bit index value h).val ≠ 0 := by
  rw [raw_set_bit_val]
  omega

/-- Helper: `(a + k*b) / b = a/b + k` when `b > 0`. -/
lemma add_mul_div (a b k : Nat) (hb : 0 < b) :
    (a + k * b) / b = a / b + k := by
  rw [Nat.add_mul_div_right _ _ hb]

/-- Tag of a packed `SetLen` record is `1`. -/
lemma raw_set_len_tag (len : Nat) (h : len ≤ Abstract.MAX_LEN) :
    tag_bits (raw_set_len len h) = 1 := by
  have h1 : len < 2 ^ 38 := by
    simp [Abstract.MAX_LEN] at h
    omega
  calc
    tag_bits (raw_set_len len h)
      = (len + 2 ^ 38) / 2 ^ 38 % 4 := by simp [tag_bits, raw_set_len_val]
    _ = (len / 2 ^ 38 + 1) % 4 := by rw [Nat.add_div_right _ (by norm_num)]
    _ = (0 + 1) % 4 := by rw [Nat.div_eq_of_lt h1]
    _ = 1 := by norm_num

/-- Reserved bits of a packed `SetLen` record are `0`. -/
lemma raw_set_len_reserved (len : Nat) (h : len ≤ Abstract.MAX_LEN) :
    reserved_bits (raw_set_len len h) = 0 := by
  have h1 : len < 2 ^ 33 := by
    simp [Abstract.MAX_LEN] at h
    omega
  calc
    reserved_bits (raw_set_len len h)
      = (len + 2 ^ 38) / 2 ^ 33 % 16 := by simp [reserved_bits, raw_set_len_val]
    _ = (len / 2 ^ 33 + 32) % 16 := by
          have : 2 ^ 38 = 32 * 2 ^ 33 := by norm_num
          rw [this]
          rw [add_mul_div _ _ _ (by norm_num)]
    _ = (0 + 32) % 16 := by rw [Nat.div_eq_of_lt h1]
    _ = 0 := by norm_num

/-- Payload of a packed `SetLen` record reconstitutes the original length. -/
lemma raw_set_len_payload (len : Nat) (h : len ≤ Abstract.MAX_LEN) :
    len_hi (raw_set_len len h) * 2 ^ 32 + payload_lo (raw_set_len len h) = len := by
  have h1 : len < 2 ^ 33 := by
    simp [Abstract.MAX_LEN] at h
    omega
  calc
    len_hi (raw_set_len len h) * 2 ^ 32 + payload_lo (raw_set_len len h)
      = ((len + 2 ^ 38) / 2 ^ 32) % 2 * 2 ^ 32 + (len + 2 ^ 38) % 2 ^ 32 := by
        simp [len_hi, payload_lo, raw_set_len_val]
    _ = (len / 2 ^ 32 + 64) % 2 * 2 ^ 32 + len % 2 ^ 32 := by
          have : 2 ^ 38 = 64 * 2 ^ 32 := by norm_num
          rw [this]
          rw [add_mul_div _ _ _ (by norm_num)]
          have : (len + 64 * 2^32) % (2^32) = len % (2^32) := by
            rw [Nat.add_mul_mod_self_right]
          rw [this]
    _ = len / 2 ^ 32 * 2 ^ 32 + len % 2 ^ 32 := by
          have : (len / 2 ^ 32 + 64) % 2 = len / 2 ^ 32 := by
            have : len / 2 ^ 32 < 2 := by
              have : len < 2 ^ 33 := h1
              omega
            omega
          rw [this]
    _ = len := by omega

/-- Tag of a packed `SetBit` record is `2`. -/
lemma raw_set_bit_tag (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    tag_bits (raw_set_bit index value h) = 2 := by
  cases value with
  | false =>
    calc
      tag_bits (raw_set_bit index false h)
        = (index + 2 ^ 39) / 2 ^ 38 % 4 := by simp [tag_bits, raw_set_bit_val]
      _ = (index / 2 ^ 38 + 2) % 4 := by
            have : 2 ^ 39 = 2 * 2 ^ 38 := by norm_num
            rw [this]
            rw [add_mul_div _ _ _ (by norm_num)]
      _ = (0 + 2) % 4 := by rw [Nat.div_eq_of_lt (by omega)]
      _ = 2 := by norm_num
  | true =>
    calc
      tag_bits (raw_set_bit index true h)
        = (index + 2 ^ 37 + 2 ^ 39) / 2 ^ 38 % 4 := by simp [tag_bits, raw_set_bit_val]
      _ = ((index + 2 ^ 37) / 2 ^ 38 + 2) % 4 := by
            have : 2 ^ 39 = 2 * 2 ^ 38 := by norm_num
            rw [this]
            rw [add_mul_div _ _ _ (by norm_num)]
      _ = (0 + 2) % 4 := by rw [Nat.div_eq_of_lt (by omega)]
      _ = 2 := by norm_num

/-- Reserved bits of a packed `SetBit` record are `0`. -/
lemma raw_set_bit_reserved (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    reserved_bits (raw_set_bit index value h) = 0 := by
  cases value with
  | false =>
    calc
      reserved_bits (raw_set_bit index false h)
        = (index + 2 ^ 39) / 2 ^ 33 % 16 := by simp [reserved_bits, raw_set_bit_val]
      _ = (index / 2 ^ 33 + 64) % 16 := by
            have : 2 ^ 39 = 64 * 2 ^ 33 := by norm_num
            rw [this]
            rw [add_mul_div _ _ _ (by norm_num)]
      _ = (0 + 64) % 16 := by rw [Nat.div_eq_of_lt (by omega)]
      _ = 0 := by norm_num
  | true =>
    calc
      reserved_bits (raw_set_bit index true h)
        = (index + 2 ^ 37 + 2 ^ 39) / 2 ^ 33 % 16 := by simp [reserved_bits, raw_set_bit_val]
      _ = (index / 2 ^ 33 + 80) % 16 := by
            rw [show index + 2 ^ 37 + 2 ^ 39 = index + 80 * 2 ^ 33 by omega]
            rw [add_mul_div _ _ _ (by norm_num)]
      _ = (0 + 80) % 16 := by rw [Nat.div_eq_of_lt (by omega)]
      _ = 0 := by norm_num

/-- Payload of a packed `SetBit` record equals the original index. -/
lemma raw_set_bit_payload (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    payload_lo (raw_set_bit index value h) = index := by
  cases value with
  | false =>
    calc
      payload_lo (raw_set_bit index false h)
        = (index + 2 ^ 39) % 2 ^ 32 := by simp [payload_lo, raw_set_bit_val]
      _ = index % 2 ^ 32 := by
            have : 2 ^ 39 = 128 * 2 ^ 32 := by norm_num
            rw [this]
            have : (index + 128 * 2^32) % (2^32) = index % (2^32) := by
              rw [Nat.add_mul_mod_self_right]
            rw [this]
      _ = index := by rw [Nat.mod_eq_of_lt h]
  | true =>
    calc
      payload_lo (raw_set_bit index true h)
        = (index + 2 ^ 37 + 2 ^ 39) % 2 ^ 32 := by simp [payload_lo, raw_set_bit_val]
      _ = index % 2 ^ 32 := by
            have : 2 ^ 37 + 2 ^ 39 = 160 * 2 ^ 32 := by norm_num
            rw [show index + 2 ^ 37 + 2 ^ 39 = index + 160 * 2 ^ 32 by omega]
            have hmod : (index + 160 * 2 ^ 32) % (2 ^ 32) = index % (2 ^ 32) := by
              rw [Nat.add_mul_mod_self_right]
            rw [hmod]
      _ = index := by rw [Nat.mod_eq_of_lt h]

/-- High length bit of a packed `SetBit` record is `0`. -/
lemma raw_set_bit_len_hi (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    len_hi (raw_set_bit index value h) = 0 := by
  cases value with
  | false =>
    calc
      len_hi (raw_set_bit index false h)
        = (index + 2 ^ 39) / 2 ^ 32 % 2 := by simp [len_hi, raw_set_bit_val]
      _ = (index / 2 ^ 32 + 128) % 2 := by
            have : 2 ^ 39 = 128 * 2 ^ 32 := by norm_num
            rw [this]
            rw [add_mul_div _ _ _ (by norm_num)]
      _ = (0 + 128) % 2 := by rw [Nat.div_eq_of_lt (by omega)]
      _ = 0 := by norm_num
  | true =>
    calc
      len_hi (raw_set_bit index true h)
        = (index + 2 ^ 37 + 2 ^ 39) / 2 ^ 32 % 2 := by simp [len_hi, raw_set_bit_val]
      _ = (index / 2 ^ 32 + 160) % 2 := by
            rw [show index + 2 ^ 37 + 2 ^ 39 = index + 160 * 2 ^ 32 by omega]
            rw [add_mul_div _ _ _ (by norm_num)]
      _ = (0 + 160) % 2 := by rw [Nat.div_eq_of_lt (by omega)]
      _ = 0 := by norm_num

/-- Value bit of a packed `SetBit` record equals the original value. -/
lemma raw_set_bit_value (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    value_bit (raw_set_bit index value h) = value := by
  cases value with
  | false =>
    change decide (((index + 2 ^ 39) / 2 ^ 37) % 2 = 1) = false
    rw [show 2 ^ 39 = 4 * 2 ^ 37 by norm_num]
    rw [add_mul_div _ _ _ (by norm_num)]
    rw [Nat.div_eq_of_lt (by omega)]
    decide
  | true =>
    change decide (((index + 2 ^ 37 + 2 ^ 39) / 2 ^ 37) % 2 = 1) = true
    rw [show index + 2 ^ 37 + 2 ^ 39 = index + 5 * 2 ^ 37 by omega]
    rw [add_mul_div _ _ _ (by norm_num)]
    rw [Nat.div_eq_of_lt (by omega)]
    decide

end PackHelpers

section RoundTrip

/-- Round-trip theorem: packing a valid `SetLen` record and unpacking yields the
same record. Corresponds to `JournalRecord::set_len` / `unpack` in `src/journal.rs`. -/
theorem pack_unpack_set_len (len : Nat) (h : len ≤ Abstract.MAX_LEN) :
    decode_record (raw_set_len len h) = some (Abstract.JournalRecord.set_len len) := by
  unfold decode_record
  rw [if_neg (raw_set_len_ne_zero len h)]
  simp only [raw_set_len_reserved, ne_eq, not_true_eq_false, ↓reduceIte,
    raw_set_len_tag, raw_set_len_payload]

/-- Round-trip theorem: packing a valid `SetBit` record and unpacking yields the
same record. Corresponds to `JournalRecord::set_bit` / `unpack` in `src/journal.rs`. -/
theorem pack_unpack_set_bit (index : Nat) (value : Bool) (h : index < 2 ^ 32) :
    decode_record (raw_set_bit index value h) =
      some (Abstract.JournalRecord.set_bit index value) := by
  unfold decode_record
  rw [if_neg (raw_set_bit_ne_zero index value h)]
  simp only [raw_set_bit_reserved, ne_eq, not_true_eq_false, ↓reduceIte,
    raw_set_bit_tag, raw_set_bit_len_hi, raw_set_bit_payload, raw_set_bit_value]

/-- Combined round-trip theorem for any valid journal record. -/
theorem pack_unpack_inverse (r : Abstract.JournalRecord) (raw : RawRecord)
    (h : pack_record r = some raw) :
    decode_record raw = some r := by
  cases r with
  | set_len len =>
      simp only [pack_record] at h
      split at h
      · next hle =>
          rw [Option.some.injEq] at h
          rw [← h]
          exact pack_unpack_set_len len hle
      · contradiction
  | set_bit index value =>
      simp only [pack_record] at h
      split at h
      · next hidx =>
          rw [Option.some.injEq] at h
          rw [← h]
          exact pack_unpack_set_bit index value hidx
      · contradiction

end RoundTrip

section Rejection

/-- Corruption detection: if reserved bits are non-zero, unpacking fails.
Mirrors the reserved-bit check in `src/journal.rs` L73-L75. -/
theorem decode_rejects_reserved (raw : RawRecord)
    (h : reserved_bits raw ≠ 0) :
    decode_record raw = none := by
  simp [decode_record, h]

/-- Corruption detection: unknown tag bits (0 or 3) are rejected.
Mirrors the tag match in `src/journal.rs` L76-L80. -/
theorem decode_rejects_unknown_tag (raw : RawRecord)
    (h1 : raw.val ≠ 0)
    (h2 : reserved_bits raw = 0)
    (h3 : tag_bits raw = 0 ∨ tag_bits raw ≥ 3) :
    decode_record raw = none := by
  have h4 : tag_bits raw < 4 := by
    simp [tag_bits]
    omega
  unfold decode_record
  split
  · contradiction
  · split
    · contradiction
    · rcases h3 with (h3 | h3)
      · simp only [h3]
      · have : tag_bits raw = 3 := by omega
        simp only [this]

/-- A `SetBit`-tagged record with the high length bit set is malformed.
Mirrors `JournalRecord::unpack` in `src/journal.rs` L84-L92. -/
theorem decode_rejects_set_bit_len_hi (raw : RawRecord)
    (h1 : raw.val ≠ 0)
    (h2 : reserved_bits raw = 0)
    (h3 : tag_bits raw = 2)
    (h4 : len_hi raw ≠ 0) :
    decode_record raw = none := by
  simp [decode_record, h1, h2, h3, h4]

/-- Every successfully decoded record is non-empty and has a supported tag with clear reserved
bits. This gives necessary conditions for arbitrary durable input, not only records produced by
`pack_record`. Mirrors `JournalRecord::unpack` in `src/journal.rs` L68-L80. -/
theorem decoded_record_has_valid_tag (raw : RawRecord) (record : Abstract.JournalRecord)
    (h : decode_record raw = some record) :
    raw.val ≠ 0 ∧ reserved_bits raw = 0 ∧ (tag_bits raw = 1 ∨ tag_bits raw = 2) := by
  have h_nonempty : raw.val ≠ 0 := by
    intro h_zero
    simp [decode_record, h_zero] at h
  have h_reserved : reserved_bits raw = 0 := by
    by_contra h_nonzero
    have h_rejected := decode_rejects_reserved raw h_nonzero
    rw [h_rejected] at h
    contradiction
  have h_tag : tag_bits raw = 1 ∨ tag_bits raw = 2 := by
    have h_lt : tag_bits raw < 4 := by
      simp [tag_bits]
      omega
    by_contra h_supported
    have h_unknown : tag_bits raw = 0 ∨ tag_bits raw ≥ 3 := by omega
    have h_rejected :=
      decode_rejects_unknown_tag raw h_nonempty h_reserved h_unknown
    rw [h_rejected] at h
    contradiction
  exact ⟨h_nonempty, h_reserved, h_tag⟩

end Rejection

end Audit.Journal
