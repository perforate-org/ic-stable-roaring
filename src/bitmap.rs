//! Roaring bitmap types backed by [`ic_stable_structures::Memory`].
//!
//! Stable-memory layout, journal packing, and crate-wide constants are documented on [`crate`].
//! This module focuses on the [`RoaringBitmap`] API: durability, checkpoint behavior, and
//! per-method costs. Application code should construct values with [`RoaringBitmap::init`].

use crate::memory::{
    MemoryReader, MemoryWriter, grow_memory_to_at_least_bytes, read_bytes, read_u64, safe_write,
    write_5_bytes, write_u64, write_zero_bytes,
};
use core::cell::{Cell, Ref, RefCell};
use core::fmt;
use ic_stable_structures::Memory;
use roaring::RoaringBitmap as RoaringHeap;

const MAGIC: [u8; 3] = *b"RSB";
const VERSION: u8 = 1;
const HEADER_SIZE: u64 = 64;
const JOURNAL_RECORD_SIZE: u64 = 5;

const MAGIC_OFFSET: u64 = 0;
const VERSION_OFFSET: u64 = 3;
const LEN_OFFSET: u64 = 4;
/// Header field: must equal [`crate::JOURNAL_CAP_SLOTS`] as `u64` (fixed journal size on disk).
const JOURNAL_SLOTS_METADATA_OFFSET: u64 = 12;
const SNAPSHOT_LEN_OFFSET: u64 = 20;

#[derive(Clone, Debug)]
struct HeapState {
    len_bits: u64,
    bitmap: RoaringHeap,
}

impl HeapState {
    fn new() -> Self {
        Self {
            len_bits: 0,
            bitmap: RoaringHeap::new(),
        }
    }
}

/// Clears all set bits with index `>= start_exclusive` (indices are `u32`).
fn remove_suffix_bits(bitmap: &mut RoaringHeap, start_exclusive: u64) {
    if start_exclusive > u32::MAX as u64 {
        return;
    }
    bitmap.remove_range(start_exclusive as u32..=u32::MAX);
}

/// Error returned when [`RoaringBitmap::init`] rejects stable memory contents.
#[derive(Debug, PartialEq, Eq)]
pub enum InitError {
    /// The first three bytes were not the expected `RSB` magic (see the [`crate`] layout section).
    BadMagic { actual: [u8; 3], expected: [u8; 3] },
    /// Header layout version is not supported by this build.
    IncompatibleVersion(u8),
    /// Catch-all for inconsistent header fields, snapshot length vs. memory size, corrupted
    /// snapshot bytes, or journal records that fail validation during replay.
    ///
    /// This includes a **journal slot count** in the header (offset `12`) that does not equal
    /// [`crate::JOURNAL_CAP_SLOTS`]—for example opening stable memory written by a build compiled
    /// with a different journal capacity.
    InvalidLayout,
    /// [`RoaringBitmap::init`] on empty memory calls [`RoaringBitmap::new`]; bootstrap failures there
    /// (usually [`BitmapError::GrowFailed`]) are returned as this variant.
    OutOfMemory,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic { actual, expected } => {
                write!(f, "bad magic number {actual:?}, expected {expected:?}")
            }
            Self::IncompatibleVersion(version) => write!(
                f,
                "unsupported layout version {version}; supported version number is {VERSION}"
            ),
            Self::InvalidLayout => write!(f, "invalid stable roaring bitmap layout"),
            Self::OutOfMemory => write!(f, "failed to allocate memory for stable roaring bitmap"),
        }
    }
}

impl std::error::Error for InitError {}

/// Error returned by [`RoaringBitmap::new`] and mutating methods (`set`, `ensure_len`, checkpoint I/O, …).
#[derive(Debug, PartialEq, Eq)]
pub enum BitmapError {
    /// `len` or `index + 1` is greater than [`crate::JOURNAL_LEN_MAX`].
    LimitsExceeded { value: u64, max: u64 },
    /// Stable memory could not be grown for a write or checkpoint.
    GrowFailed(crate::GrowFailed),
    /// Snapshot serialization or stable write failed (`roaring` / [`std::io::Write`] path).
    Io(String),
}

impl fmt::Display for BitmapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitsExceeded { value, max } => write!(
                f,
                "value {value} exceeds supported limit {max} (JOURNAL_LEN_MAX; u32 index space)"
            ),
            Self::GrowFailed(e) => write!(f, "{e}"),
            Self::Io(msg) => write!(f, "snapshot I/O: {msg}"),
        }
    }
}

impl std::error::Error for BitmapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::GrowFailed(e) => Some(e),
            _ => None,
        }
    }
}

impl From<crate::GrowFailed> for BitmapError {
    fn from(value: crate::GrowFailed) -> Self {
        Self::GrowFailed(value)
    }
}

impl From<std::io::Error> for BitmapError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

/// Start offset of the journal region in stable memory.
fn journal_offset() -> u64 {
    HEADER_SIZE
}

/// Byte offset just past the last journal slot: `64 + JOURNAL_CAP_SLOTS * 5` (not necessarily 8-aligned).
fn journal_end_bytes() -> u64 {
    journal_offset()
        .saturating_add((crate::JOURNAL_CAP_SLOTS as u64).saturating_mul(JOURNAL_RECORD_SIZE))
}

/// Start of the serialized Roaring snapshot; always 8-byte aligned after zero padding.
fn snapshot_base() -> u64 {
    let end = journal_end_bytes();
    (end + 7) & !7
}

fn read_header<M: Memory>(memory: &M) -> ([u8; 3], u8, u64, u64, u64) {
    let mut magic = [0u8; 3];
    let mut version = [0u8; 1];
    memory.read(MAGIC_OFFSET, &mut magic);
    memory.read(VERSION_OFFSET, &mut version);
    let len_bits = read_u64(memory, LEN_OFFSET);
    let journal_slots = read_u64(memory, JOURNAL_SLOTS_METADATA_OFFSET);
    let snapshot_len_bytes = read_u64(memory, SNAPSHOT_LEN_OFFSET);
    (
        magic,
        version[0],
        len_bits,
        snapshot_len_bytes,
        journal_slots,
    )
}

fn write_header<M: Memory>(
    memory: &M,
    len_bits: u64,
    snapshot_len_bytes: u64,
) -> Result<(), crate::GrowFailed> {
    safe_write(memory, MAGIC_OFFSET, &MAGIC)?;
    safe_write(memory, VERSION_OFFSET, &[VERSION])?;
    write_u64(memory, LEN_OFFSET, len_bits)?;
    write_u64(
        memory,
        JOURNAL_SLOTS_METADATA_OFFSET,
        crate::JOURNAL_CAP_SLOTS as u64,
    )?;
    write_u64(memory, SNAPSHOT_LEN_OFFSET, snapshot_len_bytes)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JournalTag {
    Empty = 0,
    SetLen = 1,
    SetBit = 2,
}

/// One journal record is **5 bytes** (40 bits, little-endian). Layout (LSB → MSB within the 40 bits):
///
/// | bits     | field        | meaning |
/// |----------|--------------|---------|
/// | 0..32    | `payload_lo` | `SetLen`: low 32 bits of `len_bits`. `SetBit`: bit index. |
/// | 32       | `len_hi`     | `SetLen`: MSB of the 33-bit length. `SetBit`: must be 0. |
/// | 33..37   | `reserved`   | must be 0 |
/// | 37       | `value`      | `SetBit`: set vs clear |
/// | 38..40   | `tag`        | 1 = SetLen, 2 = SetBit |
///
/// `SetLen` length: `len_bits = ((len_hi as u64) << 32) | (payload_lo as u64)` (33 contiguous bits).
///
/// Replay ends at the first record whose **five bytes are all zero** (unused tail).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JournalRecord([u8; 5]);

impl JournalRecord {
    fn set_len(len: u64) -> Self {
        debug_assert!(
            len <= crate::JOURNAL_LEN_MAX,
            "JournalRecord::set_len: len must be validated at API boundary"
        );
        let payload_lo = len as u32;
        let len_hi = ((len >> 32) & 1) as u32;
        Self::pack_fields(JournalTag::SetLen, false, payload_lo, len_hi)
    }

    fn set_bit(index: u32, value: bool) -> Self {
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
        let raw = raw & crate::JOURNAL_RECORD_RAW_MASK;
        let b = raw.to_le_bytes();
        Self([b[0], b[1], b[2], b[3], b[4]])
    }

    fn raw(&self) -> u64 {
        let mut w = [0u8; 8];
        w[..5].copy_from_slice(&self.0);
        u64::from_le_bytes(w) & crate::JOURNAL_RECORD_RAW_MASK
    }

    fn unpack(self) -> Result<(JournalTag, bool, u64), InitError> {
        let raw = self.raw();
        if raw == 0 {
            return Ok((JournalTag::Empty, false, 0));
        }
        let reserved = (raw >> 33) & 0xF;
        if reserved != 0 {
            return Err(InitError::InvalidLayout);
        }
        let tag_bits = (raw >> 38) & 3;
        let tag = match tag_bits {
            1 => JournalTag::SetLen,
            2 => JournalTag::SetBit,
            _ => return Err(InitError::InvalidLayout),
        };
        let value = ((raw >> 37) & 1) != 0;
        let len_hi = (raw >> 32) & 1;
        let payload_lo = raw as u32;
        let payload = match tag {
            JournalTag::SetLen => (len_hi << 32) | (payload_lo as u64),
            JournalTag::SetBit => {
                if len_hi != 0 {
                    return Err(InitError::InvalidLayout);
                }
                payload_lo as u64
            }
            JournalTag::Empty => unreachable!(),
        };
        Ok((tag, value, payload))
    }
}

/// Stable roaring bitmap with a heap mirror and a durable journal.
///
/// # Documentation split
///
/// - **[`crate`]**: on-disk layout, [`crate::JOURNAL_CAP_SLOTS`], [`crate::JOURNAL_LEN_MAX`], packed
///   journal record format, and concurrency rules shared across the crate.
/// - **`RoaringBitmap` (this type)**: logical length semantics (`len`, out-of-range `contains`),
///   what is persisted when, [`Self::init`] as the normal entry point (canister code), checkpoint
///   behavior, and method-level complexity.
///
/// # Storage model
///
/// Reads use a heap-backed [`RoaringHeap`] (`roaring` crate). Writes append **5-byte** journal
/// records (see [`crate`]) and update that mirror. A serialized roaring snapshot in stable memory
/// starts at an 8-byte aligned offset after the journal region (with up to 7 bytes of zero padding).
///
/// # Checkpointing and amortization
///
/// The journal holds at most [`crate::JOURNAL_CAP_SLOTS`] records. To ensure a mutation that returns
/// an error has not been journaled, the implementation checkpoints before an append would consume
/// the final slot. A full journal from a previous build is checkpointed before any further append.
/// That **checkpoint** costs **Θ(S)** time and I/O where **S** is the serialized snapshot size in
/// bytes (and may grow stable memory). Between checkpoints, bit mutations cost **O(1)** amortized
/// typical roaring work plus **O(1)** journal I/O per state-changing operation.
///
/// A few methods call the overflow check even when no journal append occurs, so **rare Θ(S)**
/// work can still run when opening a legacy full journal (see [`Self::set`]).
///
/// # Concurrency
///
/// Interior mutability backs the heap mirror; treat this type as **single-writer**. Do not alias
/// the same [`Memory`] through another API while an instance is live.
pub struct RoaringBitmap<M: Memory> {
    memory: M,
    state: RefCell<HeapState>,
    journal_len: Cell<u64>,
}

/// Borrowed guard over the heap mirror for batched [`Self::contains`] calls.
///
/// Obtained from [`RoaringBitmap::contains_view`]. Dropping the view ends the underlying
/// [`RefCell`] borrow acquired from [`RoaringBitmap`].
///
/// While this view is alive, other uses of the same [`RoaringBitmap`] that need to mutate the
/// heap state will contend on the `RefCell` (and may `panic!` in Rust's usual `RefCell` rules on
/// single-threaded hosts).
pub struct ContainsView<'a> {
    state: Ref<'a, HeapState>,
}

impl ContainsView<'_> {
    /// Tests membership using the [`RoaringBitmap`] length and heap mirror captured in
    /// [`RoaringBitmap::contains_view`].
    ///
    /// Out-of-range indices yield `false` (same as [`RoaringBitmap::contains`]).
    ///
    /// # Time complexity
    ///
    /// **O(1)**.
    #[inline]
    pub fn contains(&self, index: u32) -> bool {
        if u64::from(index) >= self.state.len_bits {
            return false;
        }
        self.state.bitmap.contains(index)
    }
}

impl<M: Memory> fmt::Debug for RoaringBitmap<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let st = self.state.borrow();
        f.debug_struct("RoaringBitmap")
            .field("len_bits", &st.len_bits)
            .field("cardinality", &st.bitmap.len())
            .field("journal_len", &self.journal_len.get())
            .field("journal_cap_slots", &crate::JOURNAL_CAP_SLOTS)
            .finish()
    }
}

impl<M: Memory> RoaringBitmap<M> {
    /// Writes an empty layout: header, zeroed journal padding up to the aligned snapshot base, and
    /// an empty heap mirror.
    ///
    /// **Canister code should call [`Self::init`] instead**—that is the single supported entry
    /// point for opening stable memory (including the first time, when `memory.size() == 0`, which
    /// forwards here). [`Self::new`] remains public for tests and advanced callers that need
    /// [`InitError`]-free bootstrap errors ([`BitmapError`]).
    ///
    /// # Errors
    ///
    /// Returns [`BitmapError`] when header setup or padding writes fail.
    ///
    /// # Time complexity
    ///
    /// **O(1)** with respect to bitmap contents (fixed header and journal layout).
    pub fn new(memory: M) -> Result<Self, BitmapError> {
        let snap = snapshot_base();
        grow_memory_to_at_least_bytes(&memory, snap)?;
        let journal_end = journal_end_bytes();
        write_zero_bytes(&memory, journal_offset(), journal_end - journal_offset())?;
        if journal_end < snap {
            write_zero_bytes(&memory, journal_end, snap - journal_end)?;
        }
        write_header(&memory, 0, 0)?;
        Ok(Self {
            memory,
            state: RefCell::new(HeapState::new()),
            journal_len: Cell::new(0),
        })
    }

    /// **Primary entry point:** open the bitmap from stable memory on every canister entry /
    /// reload (cold start with no pages yet, upgrade with existing bytes, or steady state).
    ///
    /// Validates the header, deserializes the roaring snapshot, reads the fixed-size journal
    /// region, and reapplies records until the first all-zero slot (see [`crate`]).
    ///
    /// When `memory.size() == 0`, this forwards to [`Self::new`] and maps any [`BitmapError`] to
    /// [`InitError::OutOfMemory`] (storage bootstrap failure).
    ///
    /// # Errors
    ///
    /// Returns [`InitError`] when magic/version/journal metadata disagree, lengths are inconsistent
    /// with the backing memory, the snapshot deserialize fails, or a journal record fails
    /// validation. In particular, the header **`journal_slots` field must match
    /// [`crate::JOURNAL_CAP_SLOTS`]**: otherwise [`InitError::InvalidLayout`] is returned—stable
    /// memory laid out under a **different compile-time journal capacity cannot be reused** without an
    /// application-level migration.
    ///
    /// # Time complexity
    ///
    /// Let **S** be the stored snapshot length in bytes and **K** the number of contiguous journal
    /// records before the first all-zero tail (**K** ≤ [`crate::JOURNAL_CAP_SLOTS`]). Decoding the
    /// snapshot costs **Θ(S)**. The journal occupies [`crate::JOURNAL_REGION_BYTES`] bytes and is
    /// read in full. Replaying **K** records costs **Σ** per-record work: typically **O(1)**
    /// amortized for `SetBit` replay, while a shrinking `SetLen` applies
    /// `remove_range`/`remove_suffix`-style work **O(C)** over roaring containers intersecting the
    /// dropped suffix (`C` ≤ number of containers in the map).
    pub fn init(memory: M) -> Result<Self, InitError> {
        if memory.size() == 0 {
            return Self::new(memory).map_err(|_| InitError::OutOfMemory);
        }
        let (magic, version, len_bits, snapshot_len_bytes, journal_slots) = read_header(&memory);
        if magic != MAGIC {
            return Err(InitError::BadMagic {
                actual: magic,
                expected: MAGIC,
            });
        }
        if version != VERSION {
            return Err(InitError::IncompatibleVersion(version));
        }
        if journal_slots != crate::JOURNAL_CAP_SLOTS as u64 {
            return Err(InitError::InvalidLayout);
        }
        let need = snapshot_base()
            .checked_add(snapshot_len_bytes)
            .ok_or(InitError::InvalidLayout)?;
        let size_bytes = memory
            .size()
            .checked_mul(crate::memory::WASM_PAGE_SIZE)
            .expect("address overflow");
        if size_bytes < need {
            return Err(InitError::InvalidLayout);
        }
        if len_bits > crate::JOURNAL_LEN_MAX {
            return Err(InitError::InvalidLayout);
        }

        let bitmap = if snapshot_len_bytes == 0 {
            RoaringHeap::new()
        } else {
            let mut reader = MemoryReader::new(&memory, snapshot_base(), snapshot_len_bytes);
            let bitmap =
                RoaringHeap::deserialize_from(&mut reader).map_err(|_| InitError::InvalidLayout)?;
            if !reader.is_exhausted() {
                return Err(InitError::InvalidLayout);
            }
            bitmap
        };
        if bitmap
            .max()
            .is_some_and(|max_index| u64::from(max_index) >= len_bits)
        {
            return Err(InitError::InvalidLayout);
        }
        let mut state = HeapState { len_bits, bitmap };

        let mut journal_len = 0u64;
        let mut saw_empty_slot = false;
        let mut chunk_buf = [0u8; crate::JOURNAL_READ_CHUNK_BYTES];
        let n_chunks = crate::JOURNAL_REGION_BYTES / crate::JOURNAL_READ_CHUNK_BYTES;
        for chunk_idx in 0..n_chunks {
            let off = journal_offset() + (chunk_idx * crate::JOURNAL_READ_CHUNK_BYTES) as u64;
            read_bytes(&memory, off, &mut chunk_buf);
            for slot in chunk_buf.chunks_exact(JOURNAL_RECORD_SIZE as usize) {
                let slot: [u8; 5] = slot.try_into().expect("chunks_exact by 5");
                if slot == [0u8; 5] {
                    saw_empty_slot = true;
                    continue;
                }
                if saw_empty_slot {
                    return Err(InitError::InvalidLayout);
                }
                apply_record(&mut state, JournalRecord(slot))?;
                journal_len += 1;
            }
        }

        Ok(Self {
            memory,
            state: RefCell::new(state),
            journal_len: Cell::new(journal_len),
        })
    }

    /// Consumes `self` and returns the [`Memory`] handle.
    ///
    /// # Time complexity
    ///
    /// **O(1)**.
    pub fn into_memory(self) -> M {
        self.memory
    }

    /// Returns the **exclusive** logical bit length (`len_bits`): valid indices are `0..len()`.
    ///
    /// This is **not** the count of set bits (cardinality).
    ///
    /// # Time complexity
    ///
    /// **O(1)**.
    pub fn len(&self) -> u64 {
        self.state.borrow().len_bits
    }

    /// Returns `true` when [`Self::len`] is zero.
    ///
    /// # Time complexity
    ///
    /// **O(1)**.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Tests whether the bit is set using the heap mirror only (stable memory is not consulted).
    ///
    /// Indices `>= len()` yield `false` without extending the bitmap.
    ///
    /// # Time complexity
    ///
    /// **O(1)**.
    #[inline]
    pub fn contains(&self, index: u32) -> bool {
        let st = self.state.borrow();
        if u64::from(index) >= st.len_bits {
            return false;
        }
        st.bitmap.contains(index)
    }

    /// Returns a borrowed view for repeated [`ContainsView::contains`] calls (see [`ContainsView`]).
    ///
    /// # Time complexity
    ///
    /// **O(1)** (one `RefCell` borrow).
    #[inline]
    pub fn contains_view(&self) -> ContainsView<'_> {
        ContainsView {
            state: self.state.borrow(),
        }
    }

    /// Grows the exclusive logical length to `min_len` if needed, without materializing unset bits.
    ///
    /// No-op when `min_len <= len()`.
    ///
    /// # Errors
    ///
    /// Returns [`BitmapError::LimitsExceeded`] when `min_len` is greater than [`crate::JOURNAL_LEN_MAX`],
    /// or other [`BitmapError`] variants when journaling or checkpointing fails. On any error, this
    /// call has not changed the logical bitmap.
    ///
    /// # Time complexity
    ///
    /// **O(1)** on the no-op path.
    ///
    /// When `min_len` is larger, this appends a `SetLen` journal record and may checkpoint; work is
    /// **O(1)** amortized for steady mutations between checkpoints, with rare **Θ(S)** checkpoints
    /// for serialized snapshot size **S**. Only growing the length does **not** enumerate unset
    /// bits in the roaring structure.
    pub fn ensure_len(&self, min_len: u64) -> Result<(), BitmapError> {
        if min_len > crate::JOURNAL_LEN_MAX {
            return Err(BitmapError::LimitsExceeded {
                value: min_len,
                max: crate::JOURNAL_LEN_MAX,
            });
        }
        let current = self.len();
        if min_len <= current {
            return Ok(());
        }
        self.append_record(JournalRecord::set_len(min_len))?;
        {
            let mut st = self.state.borrow_mut();
            st.len_bits = min_len;
        }
        Ok(())
    }

    /// Sets or clears a bit, journaling only when the logical value changes.
    ///
    /// Idempotent: if the bit already equals `value`, no journal record is appended. The
    /// `value == true` path still performs the journal-full overflow check and may therefore run a
    /// **Θ(S)** checkpoint even when the bit was already set.
    ///
    /// Setting `true` can extend [`Self::len`] to `index + 1` without a separate `SetLen` record.
    ///
    /// # Errors
    ///
    /// Returns [`BitmapError::LimitsExceeded`] if `index + 1` as `u64` would exceed [`crate::JOURNAL_LEN_MAX`]
    /// (this is unreachable for any `u32` index, but kept for API symmetry), or other [`BitmapError`]
    /// variants when journaling or checkpointing fails. On any error, this call has not changed the
    /// logical bitmap.
    ///
    /// # Time complexity
    ///
    /// **O(1)** to test [`Self::contains`] on the no-append paths.
    ///
    /// When the stored bit changes, expect **O(1)** amortized roaring updates and journal I/O, plus
    /// **Θ(S)** when a checkpoint runs (**S** = serialized snapshot size).
    pub fn set(&self, index: u32, value: bool) -> Result<(), BitmapError> {
        let need_len = u64::from(index).saturating_add(1);
        if need_len > crate::JOURNAL_LEN_MAX {
            return Err(BitmapError::LimitsExceeded {
                value: need_len,
                max: crate::JOURNAL_LEN_MAX,
            });
        }
        if value {
            if !self.contains(index) {
                self.append_record(JournalRecord::set_bit(index, true))?;
                {
                    let mut st = self.state.borrow_mut();
                    if need_len > st.len_bits {
                        st.len_bits = need_len;
                    }
                    st.bitmap.insert(index);
                }
                return Ok(());
            }
            self.maybe_checkpoint()?;
            return Ok(());
        }

        if !self.contains(index) {
            return Ok(());
        }
        self.append_record(JournalRecord::set_bit(index, false))?;
        {
            let mut st = self.state.borrow_mut();
            st.bitmap.remove(index);
        }
        Ok(())
    }

    /// Equivalent to `self.set(index, true)`. See [`Self::set`] for journaling rules and complexity.
    pub fn insert(&self, index: u32) -> Result<(), BitmapError> {
        self.set(index, true)
    }

    /// Equivalent to `self.set(index, false)`. See [`Self::set`] for journaling rules and complexity.
    pub fn clear(&self, index: u32) -> Result<(), BitmapError> {
        self.set(index, false)
    }

    /// Shrinks the exclusive logical length to `new_len`, clearing set bits at indices `>= new_len`.
    ///
    /// No-op when `new_len >= len()`.
    ///
    /// # Errors
    ///
    /// Returns [`BitmapError::LimitsExceeded`] when `new_len` is greater than [`crate::JOURNAL_LEN_MAX`],
    /// or other [`BitmapError`] variants when journaling or checkpointing fails. On any error, this
    /// call has not changed the logical bitmap.
    ///
    /// # Time complexity
    ///
    /// **O(1)** on the no-op path.
    ///
    /// Otherwise **O(C)** to clear the suffix where **C** is the number of roaring containers
    /// overlapping the removed range, plus journal append and **Θ(S)** checkpoints (**S** =
    /// serialized snapshot size) when the journal is full.
    pub fn truncate(&self, new_len: u64) -> Result<(), BitmapError> {
        if new_len > crate::JOURNAL_LEN_MAX {
            return Err(BitmapError::LimitsExceeded {
                value: new_len,
                max: crate::JOURNAL_LEN_MAX,
            });
        }
        if new_len >= self.len() {
            return Ok(());
        }
        self.append_record(JournalRecord::set_len(new_len))?;
        {
            let mut st = self.state.borrow_mut();
            st.len_bits = new_len;
            remove_suffix_bits(&mut st.bitmap, new_len);
        }
        Ok(())
    }

    /// Appends a packed mutation record to the journal.
    fn append_record(&self, record: JournalRecord) -> Result<(), BitmapError> {
        let checkpoint_before_append = crate::JOURNAL_CAP_SLOTS as u64 - 1;
        if self.journal_len.get() >= checkpoint_before_append {
            self.checkpoint()?;
        }
        let idx = self.journal_len.get();
        let base = journal_offset() + idx * JOURNAL_RECORD_SIZE;
        write_5_bytes(&self.memory, base, &record.0)?;
        self.journal_len.set(idx + 1);
        Ok(())
    }

    /// Checkpoints a full journal left by an older build or an idempotent operation.
    fn maybe_checkpoint(&self) -> Result<(), BitmapError> {
        if self.journal_len.get() >= crate::JOURNAL_CAP_SLOTS as u64 {
            self.checkpoint()?;
        }
        Ok(())
    }

    /// Writes the heap mirror back into stable memory and clears the journal.
    fn checkpoint(&self) -> Result<(), BitmapError> {
        let (len_bits, snapshot_len_bytes) = {
            let st = self.state.borrow();
            (st.len_bits, st.bitmap.serialized_size() as u64)
        };
        let need_bytes = snapshot_base()
            .checked_add(snapshot_len_bytes)
            .ok_or_else(|| BitmapError::Io("address overflow computing snapshot end".into()))?;
        grow_memory_to_at_least_bytes(&self.memory, need_bytes)?;

        {
            let st = self.state.borrow();
            let mut writer = MemoryWriter::new(&self.memory, snapshot_base());
            st.bitmap.serialize_into(&mut writer)?;
        }

        write_header(&self.memory, len_bits, snapshot_len_bytes)?;
        write_zero_bytes(
            &self.memory,
            journal_offset(),
            self.journal_len.get() * JOURNAL_RECORD_SIZE,
        )?;
        self.journal_len.set(0);
        Ok(())
    }
}

fn apply_record(state: &mut HeapState, record: JournalRecord) -> Result<(), InitError> {
    let (tag, value, payload) = record.unpack()?;
    match tag {
        JournalTag::Empty => return Err(InitError::InvalidLayout),
        JournalTag::SetLen => {
            let new_len = payload;
            if new_len > crate::JOURNAL_LEN_MAX || new_len == state.len_bits {
                return Err(InitError::InvalidLayout);
            }
            if new_len < state.len_bits {
                state.len_bits = new_len;
                remove_suffix_bits(&mut state.bitmap, new_len);
            } else {
                state.len_bits = new_len;
            }
        }
        JournalTag::SetBit => {
            if payload > u32::MAX as u64 {
                return Err(InitError::InvalidLayout);
            }
            let index = payload as u32;
            if value {
                if state.bitmap.contains(index) {
                    return Err(InitError::InvalidLayout);
                }
                let need_len = u64::from(index).saturating_add(1);
                if need_len > crate::JOURNAL_LEN_MAX {
                    return Err(InitError::InvalidLayout);
                }
                if need_len > state.len_bits {
                    state.len_bits = need_len;
                }
                state.bitmap.insert(index);
            } else {
                if u64::from(index) >= state.len_bits || !state.bitmap.contains(index) {
                    return Err(InitError::InvalidLayout);
                }
                state.bitmap.remove(index);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ic_stable_structures::{Memory, vec_mem::VectorMemory};
    use std::rc::Rc;

    #[derive(Clone)]
    struct FailOnGrowMemory {
        inner: VectorMemory,
        fail_grows: Rc<Cell<bool>>,
    }

    impl FailOnGrowMemory {
        fn new() -> Self {
            Self {
                inner: VectorMemory::default(),
                fail_grows: Rc::new(Cell::new(false)),
            }
        }

        fn fail_grows(&self) {
            self.fail_grows.set(true);
        }

        fn allow_grows(&self) {
            self.fail_grows.set(false);
        }
    }

    impl Memory for FailOnGrowMemory {
        fn size(&self) -> u64 {
            self.inner.size()
        }

        fn grow(&self, pages: u64) -> i64 {
            if self.fail_grows.get() {
                -1
            } else {
                self.inner.grow(pages)
            }
        }

        fn read(&self, offset: u64, dst: &mut [u8]) {
            self.inner.read(offset, dst);
        }

        fn write(&self, offset: u64, src: &[u8]) {
            self.inner.write(offset, src);
        }
    }

    #[cfg(journal_slots_ge_1024)]
    fn fill_journal_for_checkpoint_failure(bs: &RoaringBitmap<FailOnGrowMemory>) {
        const CONTAINER_STRIDE: u64 = 1 << 16;

        for slot in 0..(crate::JOURNAL_CAP_SLOTS as u64 - 1) {
            bs.insert((slot * CONTAINER_STRIDE) as u32).unwrap();
        }
    }

    fn reopen<M: Memory>(memory: M) -> RoaringBitmap<M> {
        RoaringBitmap::init(memory).unwrap()
    }

    #[test]
    fn limits_exceeded_returns_error() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        assert_eq!(
            bs.ensure_len(crate::JOURNAL_LEN_MAX + 1),
            Err(BitmapError::LimitsExceeded {
                value: crate::JOURNAL_LEN_MAX + 1,
                max: crate::JOURNAL_LEN_MAX,
            })
        );
        assert_eq!(
            bs.truncate(crate::JOURNAL_LEN_MAX + 1),
            Err(BitmapError::LimitsExceeded {
                value: crate::JOURNAL_LEN_MAX + 1,
                max: crate::JOURNAL_LEN_MAX,
            })
        );
    }

    #[test]
    fn fresh_create_and_reopen_roundtrip() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        assert_eq!(bs.len(), 0);
        assert!(bs.is_empty());
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert_eq!(bs.len(), 0);
        assert!(bs.is_empty());
    }

    #[test]
    fn initialization_reports_grow_failure_without_creating_a_layout() {
        let memory = FailOnGrowMemory::new();
        memory.fail_grows();
        assert!(matches!(
            RoaringBitmap::new(memory.clone()),
            Err(BitmapError::GrowFailed(_))
        ));
        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::OutOfMemory)
        ));
    }

    #[test]
    fn journal_append_uses_preallocated_memory() {
        let memory = FailOnGrowMemory::new();
        let bs = RoaringBitmap::new(memory.clone()).unwrap();
        memory.fail_grows();
        bs.insert(0).unwrap();
        memory.allow_grows();
        assert!(bs.contains(0));
        assert!(reopen(bs.into_memory()).contains(0));
    }

    #[test]
    fn insert_clear_contains_roundtrip() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        bs.insert(0).unwrap();
        bs.insert(3).unwrap();
        bs.insert(10).unwrap();
        bs.clear(3).unwrap();
        assert!(bs.contains(0));
        assert!(!bs.contains(3));
        assert!(bs.contains(10));
        assert_eq!(bs.len(), 11);
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert!(bs.contains(0));
        assert!(!bs.contains(3));
        assert!(bs.contains(10));
        assert_eq!(bs.len(), 11);
    }

    #[test]
    fn ensure_len_preserves_zero_suffix_across_reopen() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        bs.ensure_len(16).unwrap();
        assert_eq!(bs.len(), 16);
        assert!(!bs.contains(0));
        assert!(!bs.contains(15));
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert_eq!(bs.len(), 16);
        assert!(!bs.contains(0));
        assert!(!bs.contains(15));
    }

    #[test]
    fn truncate_clears_suffix_across_reopen() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        bs.insert(1).unwrap();
        bs.insert(70).unwrap();
        bs.insert(130).unwrap();
        bs.truncate(64).unwrap();
        assert_eq!(bs.len(), 64);
        assert!(bs.contains(1));
        assert!(!bs.contains(70));
        assert!(!bs.contains(130));
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert_eq!(bs.len(), 64);
        assert!(bs.contains(1));
        assert!(!bs.contains(70));
        assert!(!bs.contains(130));
    }

    #[test]
    fn checkpoint_after_full_journal_preserves_state() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        for i in 0..crate::JOURNAL_CAP_SLOTS {
            bs.insert(i as u32).unwrap();
        }
        let cleared_index = if crate::JOURNAL_CAP_SLOTS > 1 { 1 } else { 0 };
        bs.clear(cleared_index as u32).unwrap();
        bs.insert(crate::JOURNAL_CAP_SLOTS as u32).unwrap();
        assert_eq!(bs.contains(0), cleared_index != 0);
        assert!(!bs.contains(cleared_index as u32));
        assert!(bs.contains(crate::JOURNAL_CAP_SLOTS as u32));
        assert_eq!(bs.len(), (crate::JOURNAL_CAP_SLOTS + 1) as u64);
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert_eq!(bs.contains(0), cleared_index != 0);
        assert!(!bs.contains(cleared_index as u32));
        assert!(bs.contains(crate::JOURNAL_CAP_SLOTS as u32));
        assert_eq!(bs.len(), (crate::JOURNAL_CAP_SLOTS + 1) as u64);
    }

    #[test]
    fn capacity_one_mutation_avoids_double_checkpoint() {
        if crate::JOURNAL_CAP_SLOTS != 1 {
            return;
        }

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        bs.insert(7).unwrap();
        let memory = bs.into_memory();
        assert_eq!(
            read_u64(&memory, SNAPSHOT_LEN_OFFSET),
            RoaringHeap::new().serialized_size() as u64
        );
        let mut record = [0u8; JOURNAL_RECORD_SIZE as usize];
        memory.read(journal_offset(), &mut record);
        assert_ne!(record, [0u8; JOURNAL_RECORD_SIZE as usize]);
        assert!(reopen(memory).contains(7));
    }

    #[test]
    fn idempotent_insert_checkpoints_a_legacy_full_journal() {
        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let memory = bs.into_memory();
        for slot in 0..crate::JOURNAL_CAP_SLOTS as u64 {
            let record = JournalRecord::set_bit(slot as u32, true);
            write_5_bytes(
                &memory,
                journal_offset() + slot * JOURNAL_RECORD_SIZE,
                &record.0,
            )
            .unwrap();
        }

        let bs = reopen(memory);
        assert_eq!(bs.journal_len.get(), crate::JOURNAL_CAP_SLOTS as u64);
        bs.insert(0).unwrap();
        assert_eq!(bs.journal_len.get(), 0);
        assert!(reopen(bs.into_memory()).contains(0));
    }

    #[test]
    fn mixed_operations_replay_deterministically() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        bs.insert(0).unwrap();
        bs.insert(9).unwrap();
        bs.clear(0).unwrap();
        bs.ensure_len(20).unwrap();
        bs.insert(19).unwrap();
        bs.truncate(12).unwrap();
        assert_eq!(bs.len(), 12);
        assert!(!bs.contains(0));
        assert!(bs.contains(9));
        assert!(!bs.contains(19));
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert_eq!(bs.len(), 12);
        assert!(!bs.contains(0));
        assert!(bs.contains(9));
        assert!(!bs.contains(19));
    }

    #[test]
    fn sparse_high_u32_index_inserts_roundtrip() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        let a = (1u32 << 31) + 123;
        let b = u32::MAX;
        bs.insert(a).unwrap();
        bs.insert(b).unwrap();
        assert!(bs.contains(a));
        assert!(bs.contains(b));
        assert_eq!(bs.len(), u32::MAX as u64 + 1);
        let mem = bs.into_memory();
        let bs = reopen(mem);
        assert!(bs.contains(a));
        assert!(bs.contains(b));
        assert_eq!(bs.len(), u32::MAX as u64 + 1);
    }

    #[test]
    fn new_over_existing_memory_does_not_replay_stale_journal() {
        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        bs.insert(9).unwrap();
        let memory = bs.into_memory();

        let bs = RoaringBitmap::new(memory).unwrap();
        assert_eq!(bs.len(), 0);
        let memory = bs.into_memory();
        let bs = reopen(memory);
        assert_eq!(bs.len(), 0);
        assert!(!bs.contains(9));
    }

    #[test]
    fn init_rejects_snapshot_bits_outside_logical_length() {
        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        bs.insert(9).unwrap();
        bs.checkpoint().unwrap();
        let memory = bs.into_memory();
        memory.write(LEN_OFFSET, &1u64.to_le_bytes());

        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::InvalidLayout)
        ));
    }

    #[test]
    fn init_rejects_snapshot_with_trailing_bytes() {
        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        bs.insert(9).unwrap();
        bs.checkpoint().unwrap();
        let memory = bs.into_memory();
        let snapshot_len = read_u64(&memory, SNAPSHOT_LEN_OFFSET);
        write_u64(&memory, SNAPSHOT_LEN_OFFSET, snapshot_len + 1).unwrap();

        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::InvalidLayout)
        ));
    }

    #[test]
    fn init_rejects_nonzero_journal_after_empty_slot() {
        if crate::JOURNAL_CAP_SLOTS < 3 {
            return;
        }

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let memory = bs.into_memory();
        let record = JournalRecord::set_bit(7, true);

        write_5_bytes(
            &memory,
            journal_offset() + JOURNAL_RECORD_SIZE * 2,
            &record.0,
        )
        .unwrap();
        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::InvalidLayout)
        ));

        if crate::JOURNAL_REGION_BYTES > crate::JOURNAL_READ_CHUNK_BYTES {
            let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
            let memory = bs.into_memory();
            write_5_bytes(
                &memory,
                journal_offset() + crate::JOURNAL_READ_CHUNK_BYTES as u64,
                &record.0,
            )
            .unwrap();
            assert!(matches!(
                RoaringBitmap::init(memory),
                Err(InitError::InvalidLayout)
            ));
        }
    }

    #[test]
    fn init_rejects_no_op_journal_records() {
        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let memory = bs.into_memory();
        write_5_bytes(&memory, journal_offset(), &JournalRecord::set_len(0).0).unwrap();
        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::InvalidLayout)
        ));

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let memory = bs.into_memory();
        write_5_bytes(
            &memory,
            journal_offset(),
            &JournalRecord::set_bit(0, false).0,
        )
        .unwrap();
        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::InvalidLayout)
        ));

        if crate::JOURNAL_CAP_SLOTS < 2 {
            return;
        }

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let memory = bs.into_memory();
        let set_bit = JournalRecord::set_bit(0, true);
        write_5_bytes(&memory, journal_offset(), &set_bit.0).unwrap();
        write_5_bytes(&memory, journal_offset() + JOURNAL_RECORD_SIZE, &set_bit.0).unwrap();
        assert!(matches!(
            RoaringBitmap::init(memory),
            Err(InitError::InvalidLayout)
        ));
    }

    #[cfg(journal_slots_ge_1024)]
    #[test]
    fn mutation_error_does_not_apply_journaled_change() {
        const CONTAINER_STRIDE: u64 = 1 << 16;
        let requested_index = (crate::JOURNAL_CAP_SLOTS as u64 - 1) * CONTAINER_STRIDE;

        let memory = FailOnGrowMemory::new();
        let bs = RoaringBitmap::new(memory.clone()).unwrap();
        fill_journal_for_checkpoint_failure(&bs);
        let len_before = bs.len();
        memory.fail_grows();
        assert!(matches!(
            bs.insert(requested_index as u32),
            Err(BitmapError::GrowFailed(_))
        ));
        memory.allow_grows();
        assert_eq!(bs.len(), len_before);
        assert!(!bs.contains(requested_index as u32));
        let bs = reopen(bs.into_memory());
        assert_eq!(bs.len(), len_before);
        assert!(!bs.contains(requested_index as u32));

        let memory = FailOnGrowMemory::new();
        let bs = RoaringBitmap::new(memory.clone()).unwrap();
        fill_journal_for_checkpoint_failure(&bs);
        let len_before = bs.len();
        memory.fail_grows();
        assert!(matches!(
            bs.ensure_len(len_before + 1),
            Err(BitmapError::GrowFailed(_))
        ));
        memory.allow_grows();
        assert_eq!(bs.len(), len_before);
        let bs = reopen(bs.into_memory());
        assert_eq!(bs.len(), len_before);

        let memory = FailOnGrowMemory::new();
        let bs = RoaringBitmap::new(memory.clone()).unwrap();
        fill_journal_for_checkpoint_failure(&bs);
        let len_before = bs.len();
        let last_index = (len_before - 1) as u32;
        memory.fail_grows();
        assert!(matches!(
            bs.truncate(len_before - 1),
            Err(BitmapError::GrowFailed(_))
        ));
        memory.allow_grows();
        assert_eq!(bs.len(), len_before);
        assert!(bs.contains(last_index));
        let bs = reopen(bs.into_memory());
        assert_eq!(bs.len(), len_before);
        assert!(bs.contains(last_index));
    }

    #[test]
    fn invalid_magic_version_layout_and_snapshot_are_rejected() {
        let mem = VectorMemory::default();
        let bs = RoaringBitmap::new(mem).unwrap();
        bs.insert(1).unwrap();
        let mem = bs.into_memory();
        mem.write(0, b"BAD");
        assert!(matches!(
            RoaringBitmap::init(mem),
            Err(InitError::BadMagic { .. })
        ));

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let mem = bs.into_memory();
        mem.write(VERSION_OFFSET, &[VERSION.wrapping_add(1)]);
        assert!(matches!(
            RoaringBitmap::init(mem),
            Err(InitError::IncompatibleVersion(_))
        ));

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let mem = bs.into_memory();
        mem.write(JOURNAL_SLOTS_METADATA_OFFSET, &123u64.to_le_bytes());
        assert!(matches!(
            RoaringBitmap::init(mem),
            Err(InitError::InvalidLayout)
        ));

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let mem = bs.into_memory();
        mem.write(SNAPSHOT_LEN_OFFSET, &10_000_000u64.to_le_bytes());
        assert!(matches!(
            RoaringBitmap::init(mem),
            Err(InitError::InvalidLayout)
        ));
    }
}
