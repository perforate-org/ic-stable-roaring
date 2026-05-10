//! Stable-memory roaring bitmap with a heap mirror and a durable mutation journal.
//!
//! [`bitmap::RoaringBitmap`] is the primary type, and [`StableRoaringBitmap`] is a convenience
//! alias for the same implementation. The type stores the authoritative set bits in a heap-mirrored
//! [`roaring::RoaringBitmap`] (indices are `u32`), while stable memory holds a compact header, an
//! append-only journal of packed mutation records, and a serialized snapshot of the roaring structure.
//!
//! # Where documentation lives
//!
//! - **This crate root**: disk layout, shared constants (`JOURNAL_CAP_SLOTS`, `JOURNAL_READ_CHUNK_TARGET`, `JOURNAL_READ_CHUNK_MAX`; see `build.rs`, [`JOURNAL_LEN_MAX`],
//!   record packing via [`JOURNAL_RECORD_RAW_MASK`]), and cross-cutting rules (concurrency, memory
//!   ownership) that apply to every API.
//! - **[`RoaringBitmap`] / [`StableRoaringBitmap`]**: durability semantics, checkpoint behavior,
//!   idempotent `set`, **use [`RoaringBitmap::init`] in canister code** (see that method), and
//!   **per-method time/space notes** (including amortized costs when the journal fills).
//! - **[`ContainsView`]**: borrowing the heap mirror for repeated `contains` checks.
//! - **[`InitError`] / [`BitmapError`] / [`GrowFailed`]**: error meanings for `init`, mutations / [`RoaringBitmap::new`], and page growth details.
//!
//! # Design
//!
//! - **Reads** always consult the heap mirror (stable memory is not consulted for `contains`).
//! - **Mutations** append a journal record and update the heap mirror **when they change logical
//!   state** (for example, `set` to an already-matching value is a no-op). See method docs on
//!   [`RoaringBitmap`].
//! - `remove` is intentionally not part of the API.
//! - When the journal reaches capacity, the current heap state is checkpointed back into the
//!   stable snapshot and the journal is cleared.
//!
//! # Layout
//!
//! ```text
//! ---------------------------------------- <- Address 0
//! Magic `RSB`                 ↕ 3 bytes
//! ----------------------------------------
//! Layout version              ↕ 1 byte
//! ----------------------------------------
//! Length (`len_bits`)         ↕ 8 bytes
//! ----------------------------------------
//! Journal slots (fixed)       ↕ 8 bytes (`JOURNAL_CAP_SLOTS` as `u64`)
//! ----------------------------------------
//! Snapshot length             ↕ 8 bytes
//! ----------------------------------------
//! Reserved space              ↕ 36 bytes
//! ---------------------------------------- <- Address 64
//! Mutation record 0           ↕ 5 bytes
//! ----------------------------------------
//! Mutation record 1           ↕ 5 bytes
//! ----------------------------------------
//! ...
//! ----------------------------------------
//! Mutation record N-1         ↕ 5 bytes
//! ---------------------------------------- <- 64 + JOURNAL_CAP_SLOTS * 5 (not always 8-aligned)
//! Zero padding                ↕ 0..7 bytes
//! ---------------------------------------- <- snapshot_base = align_up(64 + N*5, 8)
//! Serialized Roaring snapshot bytes
//! ----------------------------------------
//! ```
//!
//! The snapshot is the canonical `RoaringBitmap` serialization (not wire-compatible with older
//! `RoaringTreemap` snapshots at the same layout version). The journal stores **5-byte** packed
//! records (40 low bits; see module-level [`JOURNAL_RECORD_RAW_MASK`]); logical lengths are
//! bounded by [`JOURNAL_LEN_MAX`]. Replay stops at the first all-zero record.
//!
//! # Durability (traps and stable memory)
//!
//! On the Internet Computer, **each message execution** runs to completion or **traps**; there is no
//! preemptive mid-instruction interrupt of your Wasm akin to arbitrary host crash. Stable memory
//! updates become visible **as your code performs `Memory::write` / `grow`**. After a method
//! returns `Ok`, callers may assume durable mutations and checkpoints **completed** for that call.
//!
//! The library does **not** try to model torn writes inside a single store: checkpoint order is
//! **write the roaring snapshot bytes, then update the header** (see [`RoaringBitmap`]).
//! [`RoaringBitmap::init`] treats a broken snapshot (deserialize failure, length mismatch) as
//! [`InitError::InvalidLayout`]. If you need stronger cross-trap guarantees between those steps,
//! consider an application-level epoch or backup region (out of scope for this crate today).
//!
//! # Type parameters
//!
//! - `M`: an [`ic_stable_structures::Memory`] implementation. The bitmap reads and writes the
//!   provided stable memory directly.
//!
//! # Concurrency
//!
//! `RoaringBitmap` uses interior mutability for the heap mirror and is intended for single-writer use.
//! The stable memory region should not be mutated through another wrapper while a bitmap instance
//! is in use.
//!
//! [`RoaringBitmap::contains_view`] holds a [`std::cell::RefCell`] read guard until dropped; calling
//! mutating methods on the same bitmap while that guard is alive will trigger `RefCell`’s runtime
//! conflict rules (typically `panic!` on the single-threaded IC). Prefer finishing the scan and
//! dropping the view before writes.
//!
//! # Example
//!
//! ```rust
//! # use ic_stable_roaring::StableRoaringBitmap;
//! # use ic_stable_structures::DefaultMemoryImpl;
//!
//! let memory = DefaultMemoryImpl::default();
//! let bitset = StableRoaringBitmap::init(memory).unwrap();
//!
//! bitset.insert(7).unwrap();
//! assert!(bitset.contains(7));
//! ```

#[cfg(feature = "canbench")]
mod bench;
pub mod bitmap;
mod memory;

// Tracked **`generated/journal_layout.rs`** is kept in sync by `build.rs`. Using
// `CARGO_MANIFEST_DIR` (not `OUT_DIR`) avoids rust-analyzer "failed to load file" before `cargo build`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/generated/journal_layout.rs"
));

/// Byte length of the on-disk journal region (`JOURNAL_CAP_SLOTS` records × 5 bytes each).
pub const JOURNAL_REGION_BYTES: usize = JOURNAL_CAP_SLOTS * 5;

const _: () = assert!(JOURNAL_REGION_BYTES.is_multiple_of(JOURNAL_READ_CHUNK_BYTES));
const _: () = assert!(JOURNAL_READ_CHUNK_BYTES.is_multiple_of(5));

/// Bit mask for one on-disk journal record: **40 low bits** of a little-endian 5-byte encoding.
pub const JOURNAL_RECORD_RAW_MASK: u64 = (1u64 << 40) - 1;

/// Maximum exclusive logical length (`len_bits`) and maximum `SetLen` value supported by the API.
///
/// Bit indices are `u32`; the exclusive logical length may be `u32::MAX + 1`.
pub const JOURNAL_LEN_MAX: u64 = (u32::MAX as u64) + 1;

pub use bitmap::RoaringBitmap as StableRoaringBitmap;
pub use bitmap::{BitmapError, ContainsView, InitError, RoaringBitmap};
pub use memory::GrowFailed;
