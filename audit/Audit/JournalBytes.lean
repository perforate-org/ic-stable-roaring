/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Journal
import Audit.SnapshotWrite

/-! # Five-byte journal representation

This file refines the numeric 40-bit journal model to the exact fixed five-byte little-endian shape
used in stable memory.

Rust references:
- `src/journal.rs` L55-L66: `JournalRecord::from_raw` and `JournalRecord::raw`
- `src/lib.rs` L17-L18: 40-bit raw mask
-/

namespace Audit.JournalBytes

/-- Rust's `[u8; 5]`: five fields make the record width explicit, while each `Fin 256` field fixes
the byte width. See `src/journal.rs` L29-L30 and L55-L66. -/
structure Bytes5 where
  b0 : SnapshotWrite.Byte
  b1 : SnapshotWrite.Byte
  b2 : SnapshotWrite.Byte
  b3 : SnapshotWrite.Byte
  b4 : SnapshotWrite.Byte
  deriving DecidableEq

/-- Two durable five-byte records are equal exactly when their five Rust array positions agree.
See the `[u8; 5]` field in `src/journal.rs` L29-L30. -/
@[ext] theorem Bytes5.ext (left right : Bytes5)
    (h0 : left.b0 = right.b0) (h1 : left.b1 = right.b1)
    (h2 : left.b2 = right.b2) (h3 : left.b3 = right.b3)
    (h4 : left.b4 = right.b4) : left = right := by
  cases left
  cases right
  simp_all

/-- The five low base-256 digits of a raw record, least significant byte first. Mirrors
`raw.to_le_bytes()` followed by selecting bytes 0 through 4 in `src/journal.rs` L55-L58. -/
def encodeRaw (raw : Journal.RawRecord) : Bytes5 := {
  b0 := ⟨raw.val % 256, Nat.mod_lt _ (by norm_num)⟩
  b1 := ⟨(raw.val / 256) % 256, Nat.mod_lt _ (by norm_num)⟩
  b2 := ⟨(raw.val / 256 ^ 2) % 256, Nat.mod_lt _ (by norm_num)⟩
  b3 := ⟨(raw.val / 256 ^ 3) % 256, Nat.mod_lt _ (by norm_num)⟩
  b4 := ⟨(raw.val / 256 ^ 4) % 256, Nat.mod_lt _ (by norm_num)⟩
}

/-- Reconstruct a 40-bit raw value from five little-endian bytes. The bound follows from the byte
types, matching zero-extension into `[u8; 8]` and `u64::from_le_bytes` in
`src/journal.rs` L60-L66. -/
def decodeBytes (bytes : Bytes5) : Journal.RawRecord :=
  ⟨bytes.b0.val + 256 * bytes.b1.val + 256 ^ 2 * bytes.b2.val +
      256 ^ 3 * bytes.b3.val + 256 ^ 4 * bytes.b4.val, by
    have h0 := bytes.b0.isLt
    have h1 := bytes.b1.isLt
    have h2 := bytes.b2.isLt
    have h3 := bytes.b3.isLt
    have h4 := bytes.b4.isLt
    norm_num
    omega⟩

/-- Five successive quotient/remainder equations reconstruct every value below `2^40`. This is the
arithmetic identity underlying the byte packing in `src/journal.rs` L55-L66. -/
lemma five_byte_reconstruct (n : Nat) (h : n < 2 ^ 40) :
    n % 256 + 256 * ((n / 256) % 256) + 256 ^ 2 * ((n / 256 ^ 2) % 256) +
      256 ^ 3 * ((n / 256 ^ 3) % 256) + 256 ^ 4 * ((n / 256 ^ 4) % 256) = n := by
  omega

/-- Persisting a raw 40-bit value to five bytes and reading it back preserves the value. Mirrors
`JournalRecord::from_raw` followed by `JournalRecord::raw` in `src/journal.rs` L55-L66. -/
theorem decode_encode_raw (raw : Journal.RawRecord) :
    decodeBytes (encodeRaw raw) = raw := by
  apply Fin.ext
  simpa [decodeBytes, encodeRaw] using five_byte_reconstruct raw.val raw.isLt

/-- Extract one base-256 digit when the lower part is smaller than its place value. This mirrors
one indexed byte selected from `u64::to_le_bytes` in `src/journal.rs` L55-L58. -/
lemma extract_byte_digit (low digit high place : Nat)
    (h_place : 0 < place) (h_low : low < place) (h_digit : digit < 256) :
    ((low + place * (digit + 256 * high)) / place) % 256 = digit := by
  rw [Nat.mul_comm place, Journal.add_mul_div _ _ _ h_place, Nat.div_eq_of_lt h_low]
  omega

/-- Every possible five-byte stable record is preserved by raw decoding followed by little-endian
encoding. Mirrors `JournalRecord::raw` followed by `JournalRecord::from_raw` in
`src/journal.rs` L55-L66. -/
theorem encode_decode_bytes (bytes : Bytes5) :
    encodeRaw (decodeBytes bytes) = bytes := by
  apply Bytes5.ext
  all_goals
    have h0 := bytes.b0.isLt
    have h1 := bytes.b1.isLt
    have h2 := bytes.b2.isLt
    have h3 := bytes.b3.isLt
    have h4 := bytes.b4.isLt
  · apply Fin.ext
    simp only [encodeRaw, decodeBytes]
    have h := extract_byte_digit 0 bytes.b0.val
      (bytes.b1.val + 256 * bytes.b2.val + 256 ^ 2 * bytes.b3.val +
        256 ^ 3 * bytes.b4.val) 1 (by norm_num) (by norm_num) h0
    norm_num at h ⊢
    omega
  · apply Fin.ext
    simp only [encodeRaw, decodeBytes]
    have h := extract_byte_digit bytes.b0.val bytes.b1.val
      (bytes.b2.val + 256 * bytes.b3.val + 256 ^ 2 * bytes.b4.val) 256
      (by norm_num) h0 h1
    norm_num at h ⊢
    omega
  · apply Fin.ext
    simp only [encodeRaw, decodeBytes]
    have h_low : bytes.b0.val + 256 * bytes.b1.val < 256 ^ 2 := by
      norm_num
      omega
    have h := extract_byte_digit
      (bytes.b0.val + 256 * bytes.b1.val) bytes.b2.val
      (bytes.b3.val + 256 * bytes.b4.val) (256 ^ 2) (by norm_num) h_low h2
    norm_num at h ⊢
    omega
  · apply Fin.ext
    simp only [encodeRaw, decodeBytes]
    have h_low : bytes.b0.val + 256 * bytes.b1.val + 256 ^ 2 * bytes.b2.val <
        256 ^ 3 := by
      norm_num
      omega
    have h := extract_byte_digit
      (bytes.b0.val + 256 * bytes.b1.val + 256 ^ 2 * bytes.b2.val)
      bytes.b3.val bytes.b4.val (256 ^ 3) (by norm_num) h_low h3
    norm_num at h ⊢
    omega
  · apply Fin.ext
    simp only [encodeRaw, decodeBytes]
    have h_low : bytes.b0.val + 256 * bytes.b1.val + 256 ^ 2 * bytes.b2.val +
        256 ^ 3 * bytes.b3.val < 256 ^ 4 := by
      norm_num
      omega
    have h := extract_byte_digit
      (bytes.b0.val + 256 * bytes.b1.val + 256 ^ 2 * bytes.b2.val +
        256 ^ 3 * bytes.b3.val) bytes.b4.val 0 (256 ^ 4)
      (by norm_num) h_low h4
    norm_num at h ⊢
    omega

/-- A valid abstract record still decodes to the same record after the exact five-byte persistence
round trip. Composes `pack_fields`, `from_raw`, `raw`, and `unpack` from
`src/journal.rs` L33-L95. -/
theorem packed_record_byte_roundtrip (record : Abstract.JournalRecord)
    (raw : Journal.RawRecord) (h : Journal.pack_record record = some raw) :
    Journal.decode_record (decodeBytes (encodeRaw raw)) = some record := by
  rw [decode_encode_raw]
  exact Journal.pack_unpack_inverse record raw h

end Audit.JournalBytes
