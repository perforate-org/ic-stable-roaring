//! Roaring bitmap types backed by the
//! [`Memory`](https://docs.rs/ic-stable-structures/latest/ic_stable_structures/trait.Memory.html)
//! trait.
//!
//! Application code should construct values with [`RoaringBitmap::init`]. This module documents
//! the stable layout; [`RoaringBitmap`] documents runtime behavior and per-method costs.
//!
//! # V1 layout
//!
//! ```text
//! ---------------------------------------- <- Address 0
//! Magic `RSB`                 ↕ 3 bytes
//! ----------------------------------------
//! Layout version              ↕ 1 byte
//! ----------------------------------------
//! Logical length (`len_bits`) ↕ 8 bytes
//! ----------------------------------------
//! Journal slot count          ↕ 8 bytes
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
//! Mutation record N - 1       ↕ 5 bytes
//! ---------------------------------------- <- 64 + JOURNAL_CAP_SLOTS * 5
//! Zero padding                ↕ 0..7 bytes
//! ---------------------------------------- <- snapshot_base = align_up(journal end, 8)
//! Serialized Roaring snapshot ↕ variable length
//! ```
//!
//! The header is 64 bytes. Journal records are packed into five bytes, and the snapshot begins at
//! the next eight-byte boundary. The snapshot uses the standard
//! [`roaring::RoaringBitmap`](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html)
//! serialization format.
//!
//! # Compatibility and recovery
//!
//! `JOURNAL_CAP_SLOTS` is stored in the header. A build with a different capacity has a
//! different journal and snapshot offset, so it cannot reopen existing memory without an
//! application-level migration; [`RoaringBitmap::init`] returns [`InitError::InvalidLayout`].
//!
//! Recovery validates the reachable header and snapshot, then replays journal records until the
//! first empty record. It deliberately does not inspect unreachable bytes after that point. The
//! caller must therefore keep the memory region isolated from untrusted writers.
//!
//! The header version describes this crate's layout, not the `roaring` crate version. Compatibility
//! with the supported Roaring serialization format is covered by a checked-in historical fixture.

use crate::journal::{JournalRecord, JournalTag};
#[cfg(test)]
use crate::memory::write_5_bytes;
use crate::memory::{
    MemoryReader, MemoryWriter, grow_memory_to_at_least_bytes, read_bytes, read_u64, safe_write,
    write_5_bytes_preallocated, write_u64, write_zero_bytes,
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
/// Header field: must equal the build-time journal capacity as `u64` (fixed journal size on disk).
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
    /// The first three bytes were not the expected `RSB` magic (see the [`crate::bitmap`] layout).
    BadMagic { actual: [u8; 3], expected: [u8; 3] },
    /// Header layout version is not supported by this build.
    IncompatibleVersion(u8),
    /// Catch-all for inconsistent header fields, snapshot length vs. memory size, corrupted
    /// snapshot bytes, or journal records that fail validation during replay.
    ///
    /// This includes a **journal slot count** in the header (offset `12`) that does not equal
    /// the build-time journal capacity—for example opening stable memory written by a build compiled
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
    /// `len` or `index + 1` exceeds the supported logical bit length.
    LimitsExceeded { value: u64, max: u64 },
    /// Stable memory could not be grown for a write or checkpoint.
    GrowFailed(crate::GrowFailed),
    /// A [`ContainsView`] still borrows the heap mirror, so a state-changing operation cannot run.
    BorrowConflict,
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
            Self::BorrowConflict => write!(f, "bitmap is borrowed by an active ContainsView"),
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
    crate::JOURNAL_END_BYTES
}

/// Start of the serialized Roaring snapshot; always 8-byte aligned after zero padding.
fn snapshot_base() -> u64 {
    crate::JOURNAL_SNAPSHOT_BASE
}

/// On-disk header for the current layout.
///
/// `journal_slots` (offset `12`, `u64`) records the build-time journal capacity this image was
/// created with. [`RoaringBitmap::init`] rejects a capacity mismatch,
/// preventing an upgraded canister with a different capacity from misinterpreting the
/// journal/snapshot layout.
#[repr(C)]
#[derive(Debug)]
struct Header {
    magic: [u8; 3],
    version: u8,
    len_bits: u64,
    journal_slots: u64,
    snapshot_len_bytes: u64,
}

impl Header {
    fn new(len_bits: u64, snapshot_len_bytes: u64) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            len_bits,
            journal_slots: crate::JOURNAL_CAP_SLOTS as u64,
            snapshot_len_bytes,
        }
    }

    fn read_fields<M: Memory>(memory: &M) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            len_bits: read_u64(memory, LEN_OFFSET),
            journal_slots: read_u64(memory, JOURNAL_SLOTS_METADATA_OFFSET),
            snapshot_len_bytes: read_u64(memory, SNAPSHOT_LEN_OFFSET),
        }
    }

    fn write<M: Memory>(&self, memory: &M) -> Result<(), crate::GrowFailed> {
        safe_write(memory, MAGIC_OFFSET, &self.magic)?;
        safe_write(memory, VERSION_OFFSET, &[self.version])?;
        write_u64(memory, LEN_OFFSET, self.len_bits)?;
        write_u64(memory, JOURNAL_SLOTS_METADATA_OFFSET, self.journal_slots)?;
        write_u64(memory, SNAPSHOT_LEN_OFFSET, self.snapshot_len_bytes)?;
        Ok(())
    }
}

fn read_header<M: Memory>(memory: &M) -> Result<Header, InitError> {
    let mut magic = [0u8; 3];
    let mut version = [0u8; 1];
    memory.read(MAGIC_OFFSET, &mut magic);
    memory.read(VERSION_OFFSET, &mut version);
    if magic != MAGIC {
        return Err(InitError::BadMagic {
            actual: magic,
            expected: MAGIC,
        });
    }
    if version[0] != VERSION {
        return Err(InitError::IncompatibleVersion(version[0]));
    }
    Ok(Header::read_fields(memory))
}

fn write_header<M: Memory>(
    memory: &M,
    len_bits: u64,
    snapshot_len_bytes: u64,
) -> Result<(), crate::GrowFailed> {
    Header::new(len_bits, snapshot_len_bytes).write(memory)
}

/// Stable roaring bitmap with a heap mirror and a durable journal.
///
/// # Documentation split
///
/// - **[`crate::bitmap`]**: on-disk layout, packed journal record format, and compatibility rules.
/// - **`RoaringBitmap` (this type)**: logical length semantics (`len`, out-of-range `contains`),
///   what is persisted when, [`Self::init`] as the normal entry point (canister code), checkpoint
///   behavior, and method-level complexity.
///
/// # Storage model
///
/// Reads use a heap-backed
/// [`roaring::RoaringBitmap`](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html).
/// Writes append **5-byte** journal records (see [`crate::bitmap`]) and update that mirror. A
/// serialized roaring snapshot in stable memory starts at an 8-byte aligned offset after the
/// journal region (with up to 7 bytes of zero padding).
///
/// # Checkpointing and amortization
///
/// The journal holds a fixed build-time number of records. To ensure a mutation that returns
/// an error has not been journaled, the implementation checkpoints before an append would consume
/// the final slot. A full journal from a previous build is checkpointed before any further append.
/// That **checkpoint** costs **Θ(S)** time and I/O where **S** is the serialized snapshot size in
/// bytes (and may grow stable memory). Between checkpoints, bit mutations cost **O(1)** amortized
/// typical roaring work plus **O(1)** journal I/O per state-changing operation.
///
/// A few methods call the overflow check even when no journal append occurs, so **rare Θ(S)**
/// work can still run when opening a legacy full journal (see [`Self::set`]).
///
/// # Failure atomicity
///
/// On ICP, mutation and checkpoint writes execute synchronously within one message execution. The
/// platform commits heap and stable-memory changes only on success and rolls them back on a trap or
/// panic; see [ICP Message Execution Property 5](https://docs.internetcomputer.org/references/message-execution-properties/).
///
/// Checkpoint serialization uses multiple [`Memory::write`] calls. This type therefore does not
/// provide generic process-crash atomicity for a custom [`Memory`] whose individual writes persist
/// across an interruption. Such an implementation must supply an equivalent rollback or
/// transactional boundary.
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
/// While this view is alive, state-changing methods on the same [`RoaringBitmap`] return
/// [`BitmapError::BorrowConflict`].
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
    /// Validates the header, deserializes the roaring snapshot, and replays journal records up to
    /// the first all-zero slot (see [`crate`]).
    ///
    /// When `memory.size() == 0`, this forwards to [`Self::new`] and maps any [`BitmapError`] to
    /// [`InitError::OutOfMemory`] (storage bootstrap failure).
    ///
    /// # Errors
    ///
    /// Returns [`InitError`] when magic/version/journal metadata disagree, lengths are inconsistent
    /// with the backing memory, the snapshot deserialize fails, or a journal record fails
    /// validation. In particular, the header **`journal_slots` field must match
    /// the build-time journal capacity**: otherwise [`InitError::InvalidLayout`] is returned—stable
    /// memory laid out under a **different compile-time journal capacity cannot be reused** without an
    /// application-level migration.
    ///
    /// # Time complexity
    ///
    /// Let **S** be the stored snapshot length in bytes and **K** the number of contiguous journal
    /// records before the first all-zero slot. Decoding the
    /// snapshot costs **Θ(S)**. Recovery reads journal chunks through the chunk containing that
    /// first empty slot. Replaying **K** records costs **Σ** per-record work: typically **O(1)**
    /// amortized for `SetBit` replay, while a shrinking `SetLen` applies
    /// `remove_range`/`remove_suffix`-style work **O(C)** over roaring containers intersecting the
    /// dropped suffix (`C` ≤ number of containers in the map).
    pub fn init(memory: M) -> Result<Self, InitError> {
        if memory.size() == 0 {
            return Self::new(memory).map_err(|_| InitError::OutOfMemory);
        }
        let header = read_header(&memory)?;
        if header.journal_slots != crate::JOURNAL_CAP_SLOTS as u64 {
            return Err(InitError::InvalidLayout);
        }
        let len_bits = header.len_bits;
        let snapshot_len_bytes = header.snapshot_len_bytes;
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
        let mut chunk_buf = [0u8; crate::JOURNAL_READ_CHUNK_BYTES];
        let n_chunks = crate::JOURNAL_REGION_BYTES / crate::JOURNAL_READ_CHUNK_BYTES;
        for chunk_idx in 0..n_chunks {
            let off = journal_offset() + (chunk_idx * crate::JOURNAL_READ_CHUNK_BYTES) as u64;
            read_bytes(&memory, off, &mut chunk_buf);
            for slot in chunk_buf.chunks_exact(JOURNAL_RECORD_SIZE as usize) {
                let slot: [u8; 5] = slot.try_into().expect("chunks_exact by 5");
                if slot == [0u8; 5] {
                    return Ok(Self {
                        memory,
                        state: RefCell::new(state),
                        journal_len: Cell::new(journal_len),
                    });
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
    /// Returns [`BitmapError::LimitsExceeded`] when `min_len` exceeds the supported logical bit length,
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
        self.ensure_mutation_not_borrowed()?;
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
    /// Returns [`BitmapError::LimitsExceeded`] if `index + 1` as `u64` would exceed the supported logical bit length
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
                self.ensure_mutation_not_borrowed()?;
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
        self.ensure_mutation_not_borrowed()?;
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
    /// Returns [`BitmapError::LimitsExceeded`] when `new_len` exceeds the supported logical bit length,
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
        self.ensure_mutation_not_borrowed()?;
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
        write_5_bytes_preallocated(&self.memory, base, &record.0);
        self.journal_len.set(idx + 1);
        Ok(())
    }

    /// Returns an error before journaling when a [`ContainsView`] holds the heap mirror.
    fn ensure_mutation_not_borrowed(&self) -> Result<(), BitmapError> {
        if self.state.try_borrow_mut().is_err() {
            return Err(BitmapError::BorrowConflict);
        }
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

#[inline]
fn apply_record(state: &mut HeapState, record: JournalRecord) -> Result<(), InitError> {
    let (tag, value, payload) = record.unpack().map_err(|_| InitError::InvalidLayout)?;
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
    use proptest::prelude::*;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    const HISTORICAL_SNAPSHOT_LEN: u64 = 262_295;
    const OPERATION_DOMAIN: u32 = 255;

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

    /// Test-only memory that records a deep copy after each production `Memory::write` call.
    /// Recording is opt-in so construction and setup writes do not enter a checkpoint trace.
    #[derive(Clone)]
    struct RecordingMemory {
        inner: VectorMemory,
        recording: Rc<Cell<bool>>,
        captures: Rc<RefCell<Vec<VectorMemory>>>,
    }

    impl RecordingMemory {
        fn new() -> Self {
            Self {
                inner: VectorMemory::default(),
                recording: Rc::new(Cell::new(false)),
                captures: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn start_recording(&self) {
            self.captures.borrow_mut().clear();
            self.recording.set(true);
        }

        fn stop_recording(&self) -> Vec<VectorMemory> {
            self.recording.set(false);
            core::mem::take(&mut *self.captures.borrow_mut())
        }
    }

    impl Memory for RecordingMemory {
        fn size(&self) -> u64 {
            self.inner.size()
        }

        fn grow(&self, pages: u64) -> i64 {
            self.inner.grow(pages)
        }

        fn read(&self, offset: u64, dst: &mut [u8]) {
            self.inner.read(offset, dst);
        }

        fn write(&self, offset: u64, src: &[u8]) {
            self.inner.write(offset, src);
            if self.recording.get() {
                let bytes = self.inner.borrow().clone();
                self.captures
                    .borrow_mut()
                    .push(Rc::new(RefCell::new(bytes)));
            }
        }
    }

    #[cfg(journal_slots_ge_1024)]
    fn fill_journal_for_checkpoint_failure(bs: &RoaringBitmap<FailOnGrowMemory>) -> u32 {
        const CONTAINER_STRIDE: u64 = 1 << 16;
        let cap = crate::JOURNAL_CAP_SLOTS as u64;
        let mut next_container = 0u64;

        loop {
            while bs.journal_len.get() < cap - 1 {
                let index = next_container * CONTAINER_STRIDE;
                bs.insert(index as u32).unwrap();
                next_container += 1;
            }

            let snapshot_len = bs.state.borrow().bitmap.serialized_size() as u64;
            let snapshot_end = snapshot_base().checked_add(snapshot_len).unwrap();
            let allocated_bytes = bs.memory.size() * crate::memory::WASM_PAGE_SIZE;
            if snapshot_end > allocated_bytes {
                let index = next_container * CONTAINER_STRIDE;
                return index as u32;
            }

            let index = next_container * CONTAINER_STRIDE;
            bs.insert(index as u32).unwrap();
            next_container += 1;
        }
    }

    fn reopen<M: Memory>(memory: M) -> RoaringBitmap<M> {
        RoaringBitmap::init(memory).unwrap()
    }

    /// Decodes the checked-in, whitespace-separated hex/RLE fixture format.
    ///
    /// Each token is either an even-length hex byte string or `hh*count`, where `hh` is one byte.
    /// Keeping the bitmap container as RLE makes the immutable 8 KiB standard-Roaring fixture
    /// inspectable in source control without asking the current `roaring` writer to reproduce it.
    fn decode_historical_snapshot() -> Vec<u8> {
        let mut bytes = Vec::new();
        for line in include_str!("../tests/fixtures/roaring-0.11.4-mixed.hex").lines() {
            let token = line.split('#').next().unwrap().trim();
            if token.is_empty() {
                continue;
            }
            let (hex, repeat) = token
                .split_once('*')
                .map_or((token, 1usize), |(hex, repeat)| {
                    (hex, repeat.parse::<usize>().unwrap())
                });
            assert!(
                hex.len().is_multiple_of(2),
                "fixture token must contain whole bytes"
            );
            let mut decoded = Vec::with_capacity(hex.len() / 2);
            for pair in hex.as_bytes().chunks_exact(2) {
                decoded.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
            }
            for _ in 0..repeat {
                bytes.extend_from_slice(&decoded);
            }
        }
        bytes
    }

    fn assert_matches_oracle(
        bitmap: &RoaringBitmap<VectorMemory>,
        expected_len: u64,
        expected_set: &BTreeSet<u32>,
    ) {
        assert_eq!(bitmap.len(), expected_len);
        for index in 0..=OPERATION_DOMAIN {
            assert_eq!(
                bitmap.contains(index),
                expected_set.contains(&index),
                "index {index}"
            );
        }
    }

    #[derive(Clone, Debug)]
    enum TestOperation {
        Insert(u32),
        Clear(u32),
        EnsureLen(u64),
        Truncate(u64),
        Reopen,
    }

    fn operation_strategy() -> impl Strategy<Value = TestOperation> {
        prop_oneof![
            (0..=OPERATION_DOMAIN).prop_map(TestOperation::Insert),
            (0..=OPERATION_DOMAIN).prop_map(TestOperation::Clear),
            (0..=u64::from(OPERATION_DOMAIN) + 1).prop_map(TestOperation::EnsureLen),
            (0..=u64::from(OPERATION_DOMAIN) + 1).prop_map(TestOperation::Truncate),
            Just(TestOperation::Reopen),
        ]
    }

    #[test]
    fn roaring_snapshot_fixture_reopens() {
        let snapshot = decode_historical_snapshot();
        assert_eq!(snapshot.len(), 8_261);
        let memory = VectorMemory::default();
        safe_write(&memory, snapshot_base(), &snapshot).unwrap();
        write_header(&memory, HISTORICAL_SNAPSHOT_LEN, snapshot.len() as u64).unwrap();

        let bitmap = RoaringBitmap::init(memory).unwrap();
        assert_eq!(bitmap.len(), HISTORICAL_SNAPSHOT_LEN);
        assert_eq!(bitmap.state.borrow().bitmap.len(), 6_009);
        for index in [
            1,
            5,
            1_000,
            1 << 16,
            (1 << 16) | 5_000,
            (2 << 16) | 100,
            (2 << 16) | 1_000,
            (3 << 16) | 7,
            (3 << 16) | 700,
            (4 << 16) | 50,
            (4 << 16) | 150,
        ] {
            assert!(bitmap.contains(index), "expected fixture bit {index}");
        }
        for index in [
            0,
            999,
            (1 << 16) | 5_001,
            (2 << 16) | 99,
            (2 << 16) | 1_001,
            (3 << 16) | 8,
            (4 << 16) | 49,
            (4 << 16) | 151,
        ] {
            assert!(!bitmap.contains(index), "unexpected fixture bit {index}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn stateful_operations_survive_reopen(operations in prop::collection::vec(operation_strategy(), 1..96)) {
            let mut bitmap = RoaringBitmap::new(VectorMemory::default()).unwrap();
            let mut expected_len = 0;
            let mut expected_set = BTreeSet::new();

            for operation in operations {
                match operation {
                    TestOperation::Insert(index) => {
                        bitmap.insert(index).unwrap();
                        expected_set.insert(index);
                        expected_len = expected_len.max(u64::from(index) + 1);
                    }
                    TestOperation::Clear(index) => {
                        bitmap.clear(index).unwrap();
                        expected_set.remove(&index);
                    }
                    TestOperation::EnsureLen(len) => {
                        bitmap.ensure_len(len).unwrap();
                        expected_len = expected_len.max(len);
                    }
                    TestOperation::Truncate(len) => {
                        bitmap.truncate(len).unwrap();
                        if len < expected_len {
                            expected_len = len;
                            expected_set.retain(|index| u64::from(*index) < len);
                        }
                    }
                    TestOperation::Reopen => {
                        bitmap = reopen(bitmap.into_memory());
                    }
                }
                assert_matches_oracle(&bitmap, expected_len, &expected_set);
            }

            bitmap = reopen(bitmap.into_memory());
            assert_matches_oracle(&bitmap, expected_len, &expected_set);
        }
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

    #[derive(Debug)]
    enum CheckpointObservation {
        Rejected,
        Accepted { len: u64, bitmap: RoaringHeap },
    }

    struct CheckpointTrace {
        expected_len: u64,
        expected_bitmap: RoaringHeap,
        observations: Vec<CheckpointObservation>,
    }

    fn checkpoint_trace<F>(seed: RoaringHeap, seed_len: u64, mutate: F) -> CheckpointTrace
    where
        F: FnOnce(&RoaringBitmap<RecordingMemory>),
    {
        let memory = RecordingMemory::new();
        drop(RoaringBitmap::new(memory.clone()).unwrap());
        let mut snapshot = Vec::with_capacity(seed.serialized_size());
        seed.serialize_into(&mut snapshot).unwrap();
        safe_write(&memory, snapshot_base(), &snapshot).unwrap();
        write_header(&memory, seed_len, snapshot.len() as u64).unwrap();

        let bs = RoaringBitmap::init(memory.clone()).unwrap();
        assert_eq!(bs.state.borrow().bitmap, seed, "bad checkpoint seed");
        mutate(&bs);
        assert!(bs.journal_len.get() > 0, "mutation was not journaled");

        let expected_len = bs.len();
        let expected_bitmap = bs.state.borrow().bitmap.clone();
        memory.start_recording();
        bs.checkpoint().unwrap();
        let captures = memory.stop_recording();

        // More than the five header fields plus journal clear proves the trace includes writes
        // issued by the real streaming Roaring serializer.
        assert!(
            captures.len() > 6,
            "checkpoint trace contained only {captures_len} writes",
            captures_len = captures.len()
        );

        let observations = captures
            .into_iter()
            .map(|captured| match RoaringBitmap::init(captured) {
                Ok(reopened) => CheckpointObservation::Accepted {
                    len: reopened.len(),
                    bitmap: reopened.state.borrow().bitmap.clone(),
                },
                Err(_) => CheckpointObservation::Rejected,
            })
            .collect();
        CheckpointTrace {
            expected_len,
            expected_bitmap,
            observations,
        }
    }

    fn assert_checkpoint_boundaries<F>(
        case: &str,
        seed: RoaringHeap,
        seed_len: u64,
        mutate: F,
    ) -> usize
    where
        F: FnOnce(&RoaringBitmap<RecordingMemory>),
    {
        let trace = checkpoint_trace(seed, seed_len, mutate);
        let mut accepted = 0;
        let mut rejected = 0;
        for (boundary, observation) in trace.observations.into_iter().enumerate() {
            match observation {
                CheckpointObservation::Accepted { len, bitmap } => {
                    accepted += 1;
                    assert_eq!(
                        len, trace.expected_len,
                        "{case}: write boundary {boundary} recovered a third logical length"
                    );
                    assert_eq!(
                        bitmap, trace.expected_bitmap,
                        "{case}: write boundary {boundary} recovered a third bitmap"
                    );
                }
                CheckpointObservation::Rejected => rejected += 1,
            }
        }
        assert!(accepted > 0, "{case}: no recoverable boundary");
        rejected
    }

    #[test]
    fn checkpoint_container_transition_boundaries_recover_current_or_reject() {
        let first = 1;
        let second = (1 << 16) | 10;
        let third = (2 << 16) | 100;
        let array_seed = RoaringHeap::from_iter([first, second, third]);
        let mut rejected = assert_checkpoint_boundaries(
            "array-to-array",
            array_seed,
            u64::from(third) + 1,
            |bs| {
                bs.clear(first).unwrap();
                bs.insert(first + 1).unwrap();
                bs.clear(second).unwrap();
                bs.insert(second + 1).unwrap();
                bs.ensure_len(u64::from(third) + 1_000).unwrap();
            },
        );

        let array_limit_seed = RoaringHeap::from_iter((0..8192).step_by(2));
        rejected += assert_checkpoint_boundaries("array-to-bitmap", array_limit_seed, 8191, |bs| {
            bs.insert(1).unwrap()
        });

        let bitmap_seed = RoaringHeap::from_iter(0..4097);
        rejected += assert_checkpoint_boundaries("bitmap-to-array", bitmap_seed, 4097, |bs| {
            bs.clear(4096).unwrap()
        });

        let mut run_seed = RoaringHeap::new();
        run_seed.insert_range(0..10_000);
        let mut run_probe = run_seed.clone();
        assert!(
            run_probe.remove_run_compression(),
            "run seed was not run-compressed"
        );
        rejected += assert_checkpoint_boundaries("run-to-run", run_seed, 10_000, |bs| {
            bs.clear(5_000).unwrap();
            bs.insert(12_000).unwrap();
        });

        assert!(rejected > 0, "suite did not exercise a rejected boundary");
    }

    #[test]
    fn checkpoint_cross_container_splice_recovers_third_state() {
        let second_key = 1u32 << 16;
        let seed = RoaringHeap::from_iter([1, 2, second_key | 10, second_key | 20]);
        let trace = checkpoint_trace(seed, u64::from(second_key | 20) + 1, |bs| {
            bs.clear(1).unwrap();
            bs.insert(second_key | 30).unwrap();
        });

        let third_bitmap = RoaringHeap::from_iter([
            second_key | 2,
            second_key | 10,
            second_key | 20,
            second_key | 30,
        ]);
        assert_ne!(third_bitmap, trace.expected_bitmap);
        let witness_boundary = trace.observations.iter().position(|observation| {
            matches!(
                observation,
                CheckpointObservation::Accepted { len, bitmap }
                    if *len == trace.expected_len && *bitmap == third_bitmap
            )
        });
        // After both new container descriptions are written, the decoder ignores the still-old
        // offset table and partitions the old payload using the new cardinalities.
        assert_eq!(witness_boundary, Some(5));
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
    fn init_stops_replay_at_first_empty_journal_slot() {
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
        let bs = reopen(memory);
        assert!(!bs.contains(7));

        if crate::JOURNAL_REGION_BYTES > crate::JOURNAL_READ_CHUNK_BYTES {
            let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
            let memory = bs.into_memory();
            write_5_bytes(
                &memory,
                journal_offset() + crate::JOURNAL_READ_CHUNK_BYTES as u64,
                &record.0,
            )
            .unwrap();
            let bs = reopen(memory);
            assert!(!bs.contains(7));
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

    #[test]
    fn active_contains_view_returns_borrow_conflict_without_journaling() {
        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let view = bs.contains_view();
        assert!(matches!(bs.ensure_len(1), Err(BitmapError::BorrowConflict)));
        drop(view);
        assert_eq!(reopen(bs.into_memory()).len(), 0);

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        let view = bs.contains_view();
        assert!(matches!(bs.insert(0), Err(BitmapError::BorrowConflict)));
        drop(view);
        assert!(!reopen(bs.into_memory()).contains(0));

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        bs.insert(0).unwrap();
        let view = bs.contains_view();
        assert!(matches!(bs.clear(0), Err(BitmapError::BorrowConflict)));
        drop(view);
        assert!(reopen(bs.into_memory()).contains(0));

        let bs = RoaringBitmap::new(VectorMemory::default()).unwrap();
        bs.insert(1).unwrap();
        let view = bs.contains_view();
        assert!(matches!(bs.truncate(1), Err(BitmapError::BorrowConflict)));
        drop(view);
        let bs = reopen(bs.into_memory());
        assert_eq!(bs.len(), 2);
        assert!(bs.contains(1));
    }

    #[cfg(journal_slots_ge_1024)]
    #[test]
    fn mutation_error_does_not_apply_journaled_change() {
        let memory = FailOnGrowMemory::new();
        let bs = RoaringBitmap::new(memory.clone()).unwrap();
        let requested_index = fill_journal_for_checkpoint_failure(&bs);
        let len_before = bs.len();
        memory.fail_grows();
        assert!(matches!(
            bs.insert(requested_index),
            Err(BitmapError::GrowFailed(_))
        ));
        memory.allow_grows();
        assert_eq!(bs.len(), len_before);
        assert!(!bs.contains(requested_index));
        let bs = reopen(bs.into_memory());
        assert_eq!(bs.len(), len_before);
        assert!(!bs.contains(requested_index));

        let memory = FailOnGrowMemory::new();
        let bs = RoaringBitmap::new(memory.clone()).unwrap();
        let requested_index = fill_journal_for_checkpoint_failure(&bs);
        let existing_index = requested_index - (1u32 << 16);
        let len_before = bs.len();
        assert!(bs.contains(existing_index));
        memory.fail_grows();
        assert!(matches!(
            bs.clear(existing_index),
            Err(BitmapError::GrowFailed(_))
        ));
        memory.allow_grows();
        assert_eq!(bs.len(), len_before);
        assert!(bs.contains(existing_index));
        let bs = reopen(bs.into_memory());
        assert_eq!(bs.len(), len_before);
        assert!(bs.contains(existing_index));

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
