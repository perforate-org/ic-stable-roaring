//! Pure encoding and decoding for the v1 five-byte journal record.
//!
//! This module deliberately has no stable-memory or roaring dependencies. Keeping the production
//! codec at this boundary makes it suitable for translation to Lean with Charon and Aeneas.

pub(crate) const RECORD_RAW_MASK: u64 = (1u64 << 40) - 1;

/// Maximum exclusive logical length and maximum `SetLen` journal payload.
pub(crate) const LEN_MAX: u64 = (u32::MAX as u64) + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JournalTag {
    Empty = 0,
    SetLen = 1,
    SetBit = 2,
}

/// A validation failure while decoding a non-empty journal record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DecodeError;

/// One journal record is **5 bytes** (40 bits, little-endian). Layout (LSB → MSB):
///
/// - bits 0..32: `SetLen` low length bits or `SetBit` index
/// - bit 32: `SetLen` high length bit; zero for `SetBit`
/// - bits 33..37: reserved and zero
/// - bit 37: `SetBit` value
/// - bits 38..40: tag (`1 = SetLen`, `2 = SetBit`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JournalRecord(pub(crate) [u8; 5]);

impl JournalRecord {
    pub(crate) fn set_len(len: u64) -> Self {
        debug_assert!(
            len <= LEN_MAX,
            "JournalRecord::set_len: len must be validated at API boundary"
        );
        let payload_lo = len as u32;
        let len_hi = ((len >> 32) & 1) as u32;
        Self::pack_fields(JournalTag::SetLen, false, payload_lo, len_hi)
    }

    pub(crate) fn set_bit(index: u32, value: bool) -> Self {
        Self::pack_fields(JournalTag::SetBit, value, index, 0)
    }

    fn pack_fields(tag: JournalTag, value: bool, payload_lo: u32, len_hi: u32) -> Self {
        let raw = (payload_lo as u64)
            | (((len_hi & 1) as u64) << 32)
            | (((value as u64) & 1) << 37)
            | (((tag as u64) & 3) << 38);
        Self::from_raw(raw)
    }

    fn from_raw(raw: u64) -> Self {
        let bytes = (raw & RECORD_RAW_MASK).to_le_bytes();
        Self([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]])
    }

    pub(crate) fn raw(self) -> u64 {
        u64::from_le_bytes([
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], 0, 0, 0,
        ])
    }

    pub(crate) fn unpack(self) -> Result<(JournalTag, bool, u64), DecodeError> {
        let raw = self.raw();
        if raw == 0 {
            return Ok((JournalTag::Empty, false, 0));
        }
        if (raw >> 33) & 0xF != 0 {
            return Err(DecodeError);
        }
        let tag = match (raw >> 38) & 3 {
            1 => JournalTag::SetLen,
            2 => JournalTag::SetBit,
            _ => return Err(DecodeError),
        };
        let value = ((raw >> 37) & 1) != 0;
        let len_hi = (raw >> 32) & 1;
        let payload_lo = raw as u32;
        let payload = match tag {
            JournalTag::SetLen => (len_hi << 32) | u64::from(payload_lo),
            JournalTag::SetBit => {
                if len_hi != 0 {
                    return Err(DecodeError);
                }
                u64::from(payload_lo)
            }
            JournalTag::Empty => unreachable!(),
        };
        Ok((tag, value, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_round_trip_at_representation_boundaries() {
        for len in [0, 1, u32::MAX as u64, LEN_MAX] {
            assert_eq!(
                JournalRecord::set_len(len).unpack(),
                Ok((JournalTag::SetLen, false, len))
            );
        }
        for index in [0, 1, u32::MAX] {
            for value in [false, true] {
                assert_eq!(
                    JournalRecord::set_bit(index, value).unpack(),
                    Ok((JournalTag::SetBit, value, u64::from(index)))
                );
            }
        }
    }

    #[test]
    fn raw_conversion_is_exact_little_endian() {
        let record = JournalRecord([0x01, 0x23, 0x45, 0x67, 0x89]);
        assert_eq!(record.raw(), 0x0089_6745_2301);
        assert_eq!(JournalRecord::from_raw(record.raw()), record);
    }

    #[test]
    fn unpack_rejects_reserved_unknown_tag_and_set_bit_high_length_bit() {
        assert_eq!(JournalRecord::from_raw(1 << 33).unpack(), Err(DecodeError));
        assert_eq!(JournalRecord::from_raw(3 << 38).unpack(), Err(DecodeError));
        assert_eq!(
            JournalRecord::from_raw((2 << 38) | (1 << 32)).unpack(),
            Err(DecodeError)
        );
    }
}
