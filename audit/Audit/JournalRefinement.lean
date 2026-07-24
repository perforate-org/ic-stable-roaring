/-
Copyright (c) 2025 ic-stable-roaring audit. All rights reserved.
Released under Apache 2.0 license as described in the file LICENSE.
Authors: ic-stable-roaring audit
-/

import Audit.Generated.JournalCore.Funs
import Audit.JournalBytes

/-! # Refinement of the generated Rust journal codec

This file is the handwritten proof boundary between the Aeneas translation of
`src/journal.rs` and the audit's small mathematical journal model.
-/

namespace Audit.JournalRefinement

open Aeneas Aeneas.Std Result
open Audit.Generated.JournalCore

/-- Forget the Aeneas array wrapper while preserving the exact five durable bytes. -/
def toBytes (record : journal.JournalRecord) : JournalBytes.Bytes5 := {
  b0 := ⟨record.val[0].val, by scalar_tac⟩
  b1 := ⟨record.val[1].val, by scalar_tac⟩
  b2 := ⟨record.val[2].val, by scalar_tac⟩
  b3 := ⟨record.val[3].val, by scalar_tac⟩
  b4 := ⟨record.val[4].val, by scalar_tac⟩
}

@[simp] lemma decode_toBytes_val (record : journal.JournalRecord) :
    (JournalBytes.decodeBytes (toBytes record)).val =
      record.val[0].val + 256 * record.val[1].val +
      256 ^ 2 * record.val[2].val + 256 ^ 3 * record.val[3].val +
      256 ^ 4 * record.val[4].val := rfl

/-- The audit byte decoder embedded in Aeneas's 64-bit scalar representation. -/
def modelRaw (record : journal.JournalRecord) : Std.U64 :=
  ⟨BitVec.ofNat 64 (JournalBytes.decodeBytes (toBytes record)).val⟩

/-- `modelRaw` does not truncate: every decoded five-byte value fits in 40 bits. -/
@[simp] lemma modelRaw_val (record : journal.JournalRecord) :
    (modelRaw record).val = (JournalBytes.decodeBytes (toBytes record)).val := by
  change (BitVec.ofNat 64 (JournalBytes.decodeBytes (toBytes record)).val).toNat = _
  rw [BitVec.toNat_ofNat, Nat.mod_eq_of_lt]
  exact Nat.lt_trans (JournalBytes.decodeBytes (toBytes record)).isLt (by norm_num)

/-- The generated Rust `raw` function is exactly the audit model's little-endian decoder. -/
@[step] theorem raw_refines (record : journal.JournalRecord) :
    Aeneas.Std.WP.spec (journal.JournalRecord.raw record) (fun raw =>
      raw = modelRaw record) := by
  unfold journal.JournalRecord.raw
  step as ⟨b0, hb0⟩
  step as ⟨b1, hb1⟩
  step as ⟨b2, hb2⟩
  step as ⟨b3, hb3⟩
  step as ⟨b4, hb4⟩
  simp only [core.num.U64.from_le_bytes, Array.make]
  subst b0
  subst b1
  subst b2
  subst b3
  subst b4
  apply (UScalar.eq_equiv_bv_eq _ _).2
  unfold modelRaw
  simp only [decode_toBytes_val]
  generalize hx0 : record.val[0] = x0
  generalize hx1 : record.val[1] = x1
  generalize hx2 : record.val[2] = x2
  generalize hx3 : record.val[3] = x3
  generalize hx4 : record.val[4] = x4
  simp only [UScalar.val]
  simp [BitVec.fromLEBytes]
  bv_tac 64

/-- A selected byte of a 64-bit little-endian decomposition is its base-256 digit. -/
lemma toLEBytes_getElem_toNat (v : BitVec 64) (i : Nat) (hi : i < 8) :
    (v.toLEBytes[i]'(by simpa using hi)).toNat =
      (v.toNat / 2 ^ (8 * i)) % 256 := by
  apply Nat.eq_of_testBit_eq
  intro j
  by_cases hj : j < 8
  · change (v.toLEBytes[i]'(by simpa using hi))[j]'hj =
      ((v.toNat / 2 ^ (8 * i)) % 256).testBit j
    rw [BitVec.getElem_toLEBytes_eq_getElem v i j (by simpa using hi) hj (by omega)]
    rw [BitVec.getElem_eq_testBit_toNat]
    rw [show 256 = 2 ^ 8 by norm_num, Nat.testBit_mod_two_pow]
    simp only [hj, decide_true, Bool.true_and]
    rw [Nat.testBit_div_two_pow]
    congr 1
    omega
  · have hleft : (v.toLEBytes[i]'(by simpa using hi)).toNat.testBit j = false :=
      Nat.testBit_eq_false_of_lt (by
        have := (v.toLEBytes[i]'(by simpa using hi)).isLt
        have : 2 ^ 8 ≤ 2 ^ j := Nat.pow_le_pow_right (by norm_num) (by omega)
        omega)
    have hright : ((v.toNat / 2 ^ (8 * i)) % 256).testBit j = false := by
      apply Nat.testBit_eq_false_of_lt
      have hmod := Nat.mod_lt (v.toNat / 2 ^ (8 * i)) (by norm_num : 0 < 256)
      have : 2 ^ 8 ≤ 2 ^ j := Nat.pow_le_pow_right (by norm_num) (by omega)
      omega
    rw [hleft, hright]

/-- The generated Rust `from_raw` keeps exactly the low 40 bits. -/
@[step] theorem from_raw_refines (raw : Std.U64) :
    Aeneas.Std.WP.spec (journal.JournalRecord.from_raw raw) (fun record =>
      (modelRaw record).val = raw.val % 2 ^ 40) := by
  unfold journal.JournalRecord.from_raw journal.RECORD_RAW_MASK
  step as ⟨maskShift, hMaskShiftVal, hMaskShiftBv⟩
  step as ⟨mask, hMaskVal, hMaskBv⟩
  step as ⟨masked, hMaskedVal, hMaskedBv⟩
  step as ⟨bytes, hbytes⟩
  rcases bytes with ⟨xs, hxs⟩
  change xs = _ at hbytes
  subst xs
  step as ⟨b0, hb0⟩
  step as ⟨b1, hb1⟩
  step as ⟨b2, hb2⟩
  step as ⟨b3, hb3⟩
  step as ⟨b4, hb4⟩
  subst b0
  subst b1
  subst b2
  subst b3
  subst b4
  simp only [modelRaw_val, decode_toBytes_val, Array.make]
  simp only [List.getElem_map, List.getElem_cons_zero, List.getElem_cons_succ,
    UScalar.val]
  change
    masked.bv.toLEBytes[0].toNat +
      256 * masked.bv.toLEBytes[1].toNat +
      256 ^ 2 * masked.bv.toLEBytes[2].toNat +
      256 ^ 3 * masked.bv.toLEBytes[3].toNat +
      256 ^ 4 * masked.bv.toLEBytes[4].toNat =
    raw.val % 2 ^ 40
  rw [toLEBytes_getElem_toNat masked.bv 0 (by norm_num),
    toLEBytes_getElem_toNat masked.bv 1 (by norm_num),
    toLEBytes_getElem_toNat masked.bv 2 (by norm_num),
    toLEBytes_getElem_toNat masked.bv 3 (by norm_num),
    toLEBytes_getElem_toNat masked.bv 4 (by norm_num)]
  norm_num
  have hmasked : masked.val = raw.val % 2 ^ 40 := by
    rw [hMaskedVal, UScalar.val_and, hMaskVal, hMaskShiftVal]
    bv_tac 64
  norm_num at hmasked ⊢
  rw [← hmasked]
  exact JournalBytes.five_byte_reconstruct masked.val (by
    rw [hmasked]
    exact Nat.mod_lt _ (by norm_num))

/-- Generated `set_bit` packing agrees with the audit's numeric layout. -/
theorem set_bit_refines (index : Std.U32) (value : Bool) :
    Aeneas.Std.WP.spec (journal.JournalRecord.set_bit index value) (fun record =>
      (modelRaw record).val = (_root_.Audit.Journal.raw_set_bit
        index.val value index.bv.isLt).val) := by
  simp only [_root_.Audit.Journal.raw_set_bit_val]
  unfold journal.JournalRecord.set_bit journal.JournalRecord.pack_fields
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  subst i
  have hi1 : i1 = 0#u32 := by
    apply UScalar.eq_of_val_eq
    rw [i1_post1, UScalar.val_and]
    norm_num
  subst i1
  subst i2
  subst i9
  have hindex32 : index.val < 2 ^ 32 := index.bv.isLt
  rw [record_post, raw1_post1, UScalar.val_or, i8_post1, UScalar.val_or,
    i4_post1, UScalar.val_or, i7_post1, i6_post1, UScalar.val_and,
    i3_post1, i11_post1, i10_post1, UScalar.val_and]
  cases value
  · have hi5 : i5 = 0#u64 := by
      apply UScalar.eq_of_val_eq
      rw [i5_post]
      norm_num
    subst i5
    have htag :
        (IScalar.hcast UScalarTy.U64
          (read_discriminant journal.JournalTag.SetBit)).val = 2 := by
      change (IScalar.hcast UScalarTy.U64 (2#isize)).val = 2
      rw [IScalar.hcast_val_eq]
      norm_num
      rfl
    rw [UScalar.cast_val_eq, UScalar.cast_val_eq, htag]
    rw [U64.size_eq]
    norm_num [Nat.shiftLeft_eq]
    rw [Nat.mod_eq_of_lt (Nat.lt_trans hindex32 (by norm_num))]
    have hindex : index.val < 2 ^ 39 :=
      Nat.lt_trans hindex32 (by norm_num)
    have hor : index.val ||| 2 ^ 39 = index.val + 2 ^ 39 := by
      calc
        index.val ||| 2 ^ 39 = 2 ^ 39 ||| index.val := Nat.or_comm _ _
        _ = 2 ^ 39 * 1 + index.val :=
          (Nat.two_pow_add_eq_or_of_lt hindex 1).symm
        _ = index.val + 2 ^ 39 := by omega
    norm_num at hor
    have htagbits :
        (2 &&& 3) * 274877906944 % 18446744073709551616 =
          549755813888 := by decide
    rw [htagbits]
    rw [hor, Nat.mod_eq_of_lt]
    · omega
  · have hi5 : i5 = 1#u64 := by
      apply UScalar.eq_of_val_eq
      rw [i5_post]
      norm_num
    subst i5
    have htag :
        (IScalar.hcast UScalarTy.U64
          (read_discriminant journal.JournalTag.SetBit)).val = 2 := by
      change (IScalar.hcast UScalarTy.U64 (2#isize)).val = 2
      rw [IScalar.hcast_val_eq]
      norm_num
      rfl
    rw [UScalar.cast_val_eq, UScalar.cast_val_eq, htag]
    rw [U64.size_eq]
    norm_num [Nat.shiftLeft_eq]
    rw [Nat.mod_eq_of_lt (Nat.lt_trans hindex32 (by norm_num))]
    have hindex37 : index.val < 2 ^ 37 :=
      Nat.lt_trans hindex32 (by norm_num)
    have hor37 : index.val ||| 2 ^ 37 = index.val + 2 ^ 37 := by
      calc
        index.val ||| 2 ^ 37 = 2 ^ 37 ||| index.val := Nat.or_comm _ _
        _ = 2 ^ 37 * 1 + index.val :=
          (Nat.two_pow_add_eq_or_of_lt hindex37 1).symm
        _ = index.val + 2 ^ 37 := by omega
    have hlow : index.val + 2 ^ 37 < 2 ^ 39 := by
      have := hindex32
      norm_num at *
      omega
    have hor39 :
        (index.val + 2 ^ 37) ||| 2 ^ 39 =
          index.val + 2 ^ 37 + 2 ^ 39 := by
      calc
        (index.val + 2 ^ 37) ||| 2 ^ 39 =
            2 ^ 39 ||| (index.val + 2 ^ 37) := Nat.or_comm _ _
        _ = 2 ^ 39 * 1 + (index.val + 2 ^ 37) :=
          (Nat.two_pow_add_eq_or_of_lt hlow 1).symm
        _ = index.val + 2 ^ 37 + 2 ^ 39 := by omega
    norm_num at hor37 hor39
    have htagbits :
        (2 &&& 3) * 274877906944 % 18446744073709551616 =
          549755813888 := by decide
    rw [htagbits]
    rw [hor37, hor39, Nat.mod_eq_of_lt]
    · omega

/-- Generated `set_len` packing agrees with the audit's numeric layout. -/
theorem set_len_refines (len : Std.U64)
    (h : len.val ≤ Abstract.MAX_LEN) :
    Aeneas.Std.WP.spec (journal.JournalRecord.set_len len) (fun record =>
      (modelRaw record).val =
        (_root_.Audit.Journal.raw_set_len len.val h).val) := by
  simp only [_root_.Audit.Journal.raw_set_len_val]
  unfold journal.JournalRecord.set_len journal.LEN_MAX
  step as ⟨max32, hmax32⟩
  step as ⟨lenMax, hLenMax⟩
  · subst max32
    rw [UScalar.cast_val_eq]
    simp [U32.rMax, U64.max, U64.numBits]
  simp [Abstract.MAX_LEN] at h
  have hlenmax : lenMax.val = Abstract.MAX_LEN := by
    rw [hLenMax]
    rw [hmax32, UScalar.cast_val_eq]
    simp [Abstract.MAX_LEN, U32.rMax]
  have hle : len ≤ lenMax := by
    change len.val ≤ lenMax.val
    rwa [hlenmax]
  simp [massert, hle]
  unfold journal.JournalRecord.pack_fields
  step as ⟨payloadLo, hPayloadLo⟩
  step as ⟨lenShift, hLenShiftVal, hLenShiftBv⟩
  step as ⟨lenBit, hLenBitVal, hLenBitBv⟩
  step as ⟨lenHi, hLenHi⟩
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  step
  change (JournalBytes.decodeBytes (toBytes record)).val =
    len.val + 2 ^ 38
  rw [← modelRaw_val, record_post, raw1_post1,
    UScalar.val_or, i8_post1, UScalar.val_or, i4_post1, UScalar.val_or,
    i7_post1, i6_post1, UScalar.val_and, i3_post1, i11_post1,
    i10_post1, UScalar.val_and]
  subst payloadLo
  subst lenHi
  subst i
  subst i2
  subst i9
  have hi5 : i5 = 0#u64 := by
    apply UScalar.eq_of_val_eq
    rw [i5_post]
    norm_num
  subst i5
  have htag :
      (IScalar.hcast UScalarTy.U64
        (read_discriminant journal.JournalTag.SetLen)).val = 1 := by
    change (IScalar.hcast UScalarTy.U64 (1#isize)).val = 1
    rw [IScalar.hcast_val_eq]
    norm_num
  rw [UScalar.cast_val_eq .U64 (UScalar.cast .U32 len),
    UScalar.cast_val_eq .U32 len,
    UScalar.cast_val_eq .U64 i1,
    i1_post1, UScalar.val_and,
    UScalar.cast_val_eq .U32 lenBit,
    hLenBitVal, UScalar.val_and, hLenShiftVal, htag]
  rw [U64.size_eq]
  norm_num [Nat.shiftLeft_eq]
  simp only [Nat.shiftRight_eq_div_pow]
  have hlen33 : len.val < 2 ^ 33 := by
    omega
  have hquot : len.val / 2 ^ 32 < 2 := by omega
  have hlow : len.val % 2 ^ 32 < 2 ^ 32 :=
    Nat.mod_lt _ (by norm_num)
  rw [Nat.mod_eq_of_lt hquot]
  have hlow64 :
      len.val % 4294967296 < 18446744073709551616 := by
    norm_num at hlow ⊢
    omega
  rw [Nat.mod_eq_of_lt hlow64]
  have hhigh64 :
      len.val / 2 ^ 32 * 4294967296 < 18446744073709551616 := by
    have := hquot
    norm_num at *
    omega
  rw [Nat.mod_eq_of_lt hhigh64]
  have hjoin :
      len.val % 2 ^ 32 ||| len.val / 2 ^ 32 * 2 ^ 32 = len.val := by
    calc
      len.val % 2 ^ 32 ||| len.val / 2 ^ 32 * 2 ^ 32 =
          len.val / 2 ^ 32 * 2 ^ 32 ||| len.val % 2 ^ 32 := Nat.or_comm _ _
      _ = 2 ^ 32 * (len.val / 2 ^ 32) + len.val % 2 ^ 32 := by
        rw [Nat.mul_comm]
        exact (Nat.two_pow_add_eq_or_of_lt hlow _).symm
      _ = len.val := by omega
  norm_num at hjoin
  norm_num
  rw [hjoin]
  have htagJoin : len.val ||| 2 ^ 38 = len.val + 2 ^ 38 := by
    calc
      len.val ||| 2 ^ 38 = 2 ^ 38 ||| len.val := Nat.or_comm _ _
      _ = 2 ^ 38 * 1 + len.val :=
        (Nat.two_pow_add_eq_or_of_lt (i := 38)
          (Nat.lt_trans hlen33 (by norm_num)) 1).symm
      _ = len.val + 2 ^ 38 := by omega
  norm_num at htagJoin
  rw [htagJoin, Nat.mod_eq_of_lt]
  omega

/-- Interpret the generated Rust decoder result in the audit's abstract journal type. -/
def toAbstract : core.result.Result
    (journal.JournalTag × Bool × Std.U64) journal.DecodeError →
    Option Abstract.JournalRecord
  | .Ok (journal.JournalTag.Empty, _, _) => none
  | .Ok (journal.JournalTag.SetLen, _, payload) =>
      some (.set_len payload.val)
  | .Ok (journal.JournalTag.SetBit, value, payload) =>
      some (.set_bit payload.val value)
  | .Err _ => none

/-- Generated Rust validation and field decoding agree with the numeric audit model. -/
theorem unpack_refines (record : journal.JournalRecord) :
    Aeneas.Std.WP.spec (journal.JournalRecord.unpack record) (fun decoded =>
      toAbstract decoded = _root_.Audit.Journal.decode_record
        (JournalBytes.decodeBytes (toBytes record))) := by
  unfold journal.JournalRecord.unpack
  step with raw_refines as ⟨raw, hraw⟩
  rw [hraw]
  have hval := modelRaw_val record
  unfold _root_.Audit.Journal.decode_record
  split
  · next hz =>
      have hz' : (JournalBytes.decodeBytes (toBytes record)).val = 0 := by
        rw [← hval]
        simpa using congrArg UScalar.val hz
      simp [toAbstract, hz']
  · next hnz =>
      have hnz' : (JournalBytes.decodeBytes (toBytes record)).val ≠ 0 := by
        intro hzero
        apply hnz
        apply (UScalar.eq_equiv_bv_eq _ _).2
        apply BitVec.eq_of_toNat_eq
        simpa [modelRaw_val, hzero]
      step as ⟨reservedShift, hReservedShiftVal, hReservedShiftBv⟩
      step as ⟨reserved, hReservedVal, hReservedBv⟩
      have hreserved : reserved.val =
          _root_.Audit.Journal.reserved_bits
            (JournalBytes.decodeBytes (toBytes record)) := by
        rw [hReservedVal, UScalar.val_and, hReservedShiftVal, hval]
        simp only [_root_.Audit.Journal.reserved_bits, Nat.shiftRight_eq_div_pow]
        norm_num
        have hand := Nat.and_two_pow_sub_one_eq_mod
          ((JournalBytes.decodeBytes (toBytes record)).val / 2 ^ 33) 4
        norm_num at hand
        exact hand
      split
      · next hnonzero =>
          have hreserved_ne : reserved.val ≠ 0 := by
            intro hz
            have heq : reserved = 0#u64 := by scalar_tac
            simp [heq] at hnonzero
          have hmodel_ne : _root_.Audit.Journal.reserved_bits
              (JournalBytes.decodeBytes (toBytes record)) ≠ 0 := by
            rwa [← hreserved]
          simp [toAbstract, hmodel_ne]
      · next hzero =>
          have hreserved_zero : reserved.val = 0 := by
            have heq : reserved = 0#u64 := by
              by_contra hne
              apply hzero
              simp [hne]
            simp [heq]
          have hmodel_zero : _root_.Audit.Journal.reserved_bits
              (JournalBytes.decodeBytes (toBytes record)) = 0 := by
            rw [← hreserved]
            exact hreserved_zero
          step as ⟨tagShift, hTagShiftVal, hTagShiftBv⟩
          step as ⟨tag, hTagVal, hTagBv⟩
          have htag : tag.val = _root_.Audit.Journal.tag_bits
              (JournalBytes.decodeBytes (toBytes record)) := by
            rw [hTagVal, UScalar.val_and, hTagShiftVal, hval]
            simp only [_root_.Audit.Journal.tag_bits, Nat.shiftRight_eq_div_pow]
            norm_num
            have hand := Nat.and_two_pow_sub_one_eq_mod
              ((JournalBytes.decodeBytes (toBytes record)).val / 2 ^ 38) 2
            norm_num at hand
            exact hand
          simp only [hmodel_zero, ne_eq, not_true_eq_false, ↓reduceIte]
          have htag_lt : _root_.Audit.Journal.tag_bits
              (JournalBytes.decodeBytes (toBytes record)) < 4 := by
            exact Nat.mod_lt _ (by norm_num)
          have htag_cases : _root_.Audit.Journal.tag_bits
                (JournalBytes.decodeBytes (toBytes record)) = 0 ∨
              _root_.Audit.Journal.tag_bits
                (JournalBytes.decodeBytes (toBytes record)) = 1 ∨
              _root_.Audit.Journal.tag_bits
                (JournalBytes.decodeBytes (toBytes record)) = 2 ∨
              _root_.Audit.Journal.tag_bits
                (JournalBytes.decodeBytes (toBytes record)) = 3 := by
            omega
          rcases htag_cases with htag0 | htag1 | htag2 | htag3
          · have heq : tag = 0#u64 := by scalar_tac
            simp only [heq, htag0]
            change Aeneas.Std.WP.spec (ok (core.result.Result.Err ())) (fun decoded => _)
            simp [toAbstract]
          · have heq : tag = 1#u64 := by scalar_tac
            simp only [heq, htag1]
            change Aeneas.Std.WP.spec (do
              let valueShift ← modelRaw record >>> 37#i32
              let value ← lift (valueShift &&& 1#u64)
              let lenShift ← modelRaw record >>> 32#i32
              let lenHi ← lift (lenShift &&& 1#u64)
              let payloadLo ← lift (UScalar.cast .U32 (modelRaw record))
              let shiftedHi ← lenHi <<< 32#i32
              let widenedLo ← lift (core.convert.num.FromU64U32.from payloadLo)
              let payload ← lift (shiftedHi ||| widenedLo)
              ok (core.result.Result.Ok
                (journal.JournalTag.SetLen, value != 0#u64, payload))) (fun decoded => _)
            step as ⟨valueShift, hValueShiftVal, hValueShiftBv⟩
            step as ⟨value, hValueVal, hValueBv⟩
            step as ⟨lenShift, hLenShiftVal, hLenShiftBv⟩
            step as ⟨lenHi, hLenHiVal, hLenHiBv⟩
            step as ⟨payloadLo, hPayloadLo⟩
            have hlen : lenHi.val = _root_.Audit.Journal.len_hi
                (JournalBytes.decodeBytes (toBytes record)) := by
              rw [hLenHiVal, UScalar.val_and, hLenShiftVal, hval]
              simp only [_root_.Audit.Journal.len_hi, Nat.shiftRight_eq_div_pow]
              norm_num
            have hlo : payloadLo.val = _root_.Audit.Journal.payload_lo
                (JournalBytes.decodeBytes (toBytes record)) := by
              rw [hPayloadLo, UScalar.cast_val_eq, hval]
              simp [_root_.Audit.Journal.payload_lo]
            step as ⟨shiftedHi, hShiftedHiVal, hShiftedHiBv⟩
            simp only [lift, core.convert.num.FromU64U32.from, Bind.bind,
              Aeneas.Std.bind, Aeneas.Std.WP.spec, Aeneas.Std.WP.theta,
              Aeneas.Std.WP.wp_return, toAbstract]
            simp only [Option.some.injEq,
              Abstract.JournalRecord.set_len.injEq, UScalar.val_or]
            rw [hShiftedHiVal]
            have hwide : (BitVec.setWidth UScalarTy.U64.numBits
                payloadLo.bv#uscalar).val = payloadLo.val := by
              simpa only [core.convert.num.FromU64U32.from, UScalar.val] using
                Aeneas.Std.core.convert.num.FromU64U32.from_val_eq payloadLo
            rw [hwide]
            have hshift : lenHi.val <<< 32 % U64.size = lenHi.val <<< 32 := by
              rw [Nat.mod_eq_of_lt]
              have hlen_bound : lenHi.val < 2 := by
                rw [hlen]
                exact Nat.mod_lt _ (by norm_num)
              scalar_tac
            rw [hshift,
              ← Nat.shiftLeft_add_eq_or_of_lt (by scalar_tac : payloadLo.val < 2 ^ 32)]
            simp only [Nat.shiftLeft_eq, hlen, hlo]
          · have heq : tag = 2#u64 := by scalar_tac
            simp only [heq, htag2]
            change Aeneas.Std.WP.spec (do
              let valueShift ← modelRaw record >>> 37#i32
              let value ← lift (valueShift &&& 1#u64)
              let lenShift ← modelRaw record >>> 32#i32
              let lenHi ← lift (lenShift &&& 1#u64)
              let payloadLo ← lift (UScalar.cast .U32 (modelRaw record))
              if (lenHi != 0#u64) = true then ok (core.result.Result.Err ())
              else do
                let payload ← lift (core.convert.num.FromU64U32.from payloadLo)
                ok (core.result.Result.Ok
                  (journal.JournalTag.SetBit, value != 0#u64, payload))) (fun decoded => _)
            step as ⟨valueShift, hValueShiftVal, hValueShiftBv⟩
            step as ⟨value, hValueVal, hValueBv⟩
            step as ⟨lenShift, hLenShiftVal, hLenShiftBv⟩
            step as ⟨lenHi, hLenHiVal, hLenHiBv⟩
            step as ⟨payloadLo, hPayloadLo⟩
            have hlen : lenHi.val = _root_.Audit.Journal.len_hi
                (JournalBytes.decodeBytes (toBytes record)) := by
              rw [hLenHiVal, UScalar.val_and, hLenShiftVal, hval]
              simp only [_root_.Audit.Journal.len_hi, Nat.shiftRight_eq_div_pow]
              norm_num
            have hlo : payloadLo.val = _root_.Audit.Journal.payload_lo
                (JournalBytes.decodeBytes (toBytes record)) := by
              rw [hPayloadLo, UScalar.cast_val_eq, hval]
              simp [_root_.Audit.Journal.payload_lo]
            have hvalueNat : value.val =
                ((JournalBytes.decodeBytes (toBytes record)).val / 2 ^ 37) % 2 := by
              rw [hValueVal, UScalar.val_and, hValueShiftVal, hval]
              simp only [Nat.shiftRight_eq_div_pow]
              norm_num
            have hvalue : (value != 0#u64) = _root_.Audit.Journal.value_bit
                (JournalBytes.decodeBytes (toBytes record)) := by
              simp only [_root_.Audit.Journal.value_bit]
              have hbit : ((JournalBytes.decodeBytes (toBytes record)).val /
                  2 ^ 37) % 2 < 2 := Nat.mod_lt _ (by norm_num)
              have hcases : value.val = 0 ∨ value.val = 1 := by omega
              rcases hcases with hv | hv
              · have heq : value = 0#u64 := by scalar_tac
                have hbit_zero :
                    ((JournalBytes.decodeBytes (toBytes record)).val /
                      2 ^ 37) % 2 = 0 := by omega
                rw [heq, hbit_zero]
                decide
              · have heq : value = 1#u64 := by scalar_tac
                have hbit_one :
                    ((JournalBytes.decodeBytes (toBytes record)).val /
                      2 ^ 37) % 2 = 1 := by omega
                rw [heq, hbit_one]
                decide
            split
            · next hnonzero =>
                have hlen_ne : lenHi.val ≠ 0 := by
                  intro hz
                  have heq : lenHi = 0#u64 := by scalar_tac
                  simp [heq] at hnonzero
                have hmodel_ne : _root_.Audit.Journal.len_hi
                    (JournalBytes.decodeBytes (toBytes record)) ≠ 0 := by
                  rwa [← hlen]
                simp [toAbstract, hmodel_ne]
            · next hzero =>
                have hlen_zero : lenHi.val = 0 := by
                  have heq : lenHi = 0#u64 := by
                    by_contra hne
                    apply hzero
                    simp [hne]
                  simp [heq]
                have hmodel_zero : _root_.Audit.Journal.len_hi
                    (JournalBytes.decodeBytes (toBytes record)) = 0 := by
                  rwa [← hlen]
                simp only [lift, core.convert.num.FromU64U32.from, Bind.bind,
                  Aeneas.Std.bind, Aeneas.Std.WP.spec, Aeneas.Std.WP.theta,
                  Aeneas.Std.WP.wp_return, toAbstract, hmodel_zero,
                  not_true_eq_false, ↓reduceIte, Option.some.injEq,
                  Abstract.JournalRecord.set_bit.injEq]
                constructor
                · calc
                    (BitVec.setWidth UScalarTy.U64.numBits
                        payloadLo.bv#uscalar).val
                        = payloadLo.val := by
                          simpa only [core.convert.num.FromU64U32.from,
                            UScalar.val] using
                            Aeneas.Std.core.convert.num.FromU64U32.from_val_eq
                              payloadLo
                    _ = _ := hlo
                · exact hvalue
          · have heq : tag = 3#u64 := by scalar_tac
            simp only [heq, htag3]
            change Aeneas.Std.WP.spec (ok (core.result.Result.Err ())) (fun decoded => _)
            simp [toAbstract]

end Audit.JournalRefinement
