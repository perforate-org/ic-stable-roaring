#![doc = include_str!("../README.md")]

#[cfg(feature = "canbench")]
mod bench;
pub mod bitmap;
mod memory;

// `build.rs` generates this file under Cargo's `OUT_DIR` for the active build configuration.
include!(concat!(env!("OUT_DIR"), "/journal_layout.rs"));

/// Byte length of the on-disk journal region (`JOURNAL_CAP_SLOTS` records × 5 bytes each).
pub(crate) const JOURNAL_REGION_BYTES: usize = JOURNAL_CAP_SLOTS * 5;

const _: () = assert!(JOURNAL_REGION_BYTES.is_multiple_of(JOURNAL_READ_CHUNK_BYTES));
const _: () = assert!(JOURNAL_READ_CHUNK_BYTES.is_multiple_of(5));

/// Bit mask for one on-disk journal record: **40 low bits** of a little-endian 5-byte encoding.
pub(crate) const JOURNAL_RECORD_RAW_MASK: u64 = (1u64 << 40) - 1;

/// Maximum exclusive logical length (`len_bits`) and maximum `SetLen` value supported by the API.
///
/// Bit indices are `u32`; the exclusive logical length may be `u32::MAX + 1`.
pub(crate) const JOURNAL_LEN_MAX: u64 = (u32::MAX as u64) + 1;

pub use bitmap::RoaringBitmap as StableRoaringBitmap;
pub use bitmap::{BitmapError, ContainsView, InitError, RoaringBitmap};
pub use memory::GrowFailed;
