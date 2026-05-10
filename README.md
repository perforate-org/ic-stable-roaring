# ic-stable-roaring

Stable-memory [Roaring Bitmap](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html) for [Internet Computer](https://internetcomputer.org/) canisters. Reads use a heap mirror; `set`, `insert`, `clear`, `ensure_len`, and `truncate` persist through an append-only journal and occasional snapshots into stable memory.

**Durability:** On the IC, a canister call runs to completion or traps; after a mutating method returns `Ok`, its stable writes for that call have finished. This crate does not add a separate crash-recovery protocol beyond validating bytes on `init` (`InvalidLayout` if the snapshot is inconsistent). See **`cargo doc`** on the `ic-stable-roaring` crate for the full **Durability** section.

## Documentation

| Location                                                         | What to read there                                                                                                                                                                           |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **This README**                                                  | What the crate does, how to get started, operational caveats.                                                                                                                                |
| **Crate docs** (`cargo doc --open`, then `ic_stable_roaring`)    | On-disk **layout**, journal packing, **constants** (`JOURNAL_CAP_SLOTS` / `JOURNAL_READ_CHUNK_TARGET` / `JOURNAL_READ_CHUNK_MAX` in `build.rs`, `JOURNAL_LEN_MAX`, …), and crate-wide rules. |
| **Type & method docs** (`StableRoaringBitmap` / `RoaringBitmap`) | Durability, **checkpointing**, **per-method time bounds**, **`init` as the normal constructor**, and errors (`BitmapError`, `InitError`).                                                    |

Complexity and edge cases (for example checkpoint cost **Θ(S)** when the journal fills, or **O(C)** work when truncating a suffix) live on those API docs—see **Time complexity** on each method.

## Operations (overview)

- **`contains`** — heap mirror only; out-of-range indices are `false`.
- **`set` / `insert` / `clear`** — journal + heap update **when the logical bit changes** (no journal record on a no-op); see the `set` docs for rare checkpoint behavior even on idempotent sets.
- **`ensure_len`** — extends the exclusive logical length without materializing zero bits (capped by `JOURNAL_LEN_MAX`).
- **`truncate`** — shortens the logical length and clears set bits from the new end onward (same cap).
- **`len` / `is_empty`** — **logical** length, not set-bit cardinality.

Mutations return **`Result<..., BitmapError>`** (`LimitsExceeded`, `GrowFailed`, snapshot **I/O**). Lengths above **`JOURNAL_LEN_MAX`** are reported as `LimitsExceeded`, not panics.

The on-disk journal records **logical state changes**, not every redundant `set` call—do not treat it as a verbatim audit log. Journal capacity is fixed when the crate is built (**`JOURNAL_CAP_SLOTS`**, default `8192`). At compile time, set **`JOURNAL_CAP_SLOTS`** to any positive **`usize`**. **`JOURNAL_READ_CHUNK_BYTES`** is chosen in **`build.rs`** as the **largest divisor** of **`JOURNAL_CAP_SLOTS * 5`** that is a multiple of **`5`** and does not exceed **`floor5(min(JOURNAL_CAP_SLOTS * 5, JOURNAL_READ_CHUNK_TARGET, JOURNAL_READ_CHUNK_MAX))`** (defaults: target **`5120`**, max **`32768`**). The **target** defaults to the historical **`5120`** so an **empty or short journal** still pays only a **small first `Memory::read`**, while **`JOURNAL_READ_CHUNK_MAX`** remains a hard stack / buffer ceiling. Set **`JOURNAL_READ_CHUNK_TARGET`** high (up to the region size) if you intentionally want **fewer, larger** reads when the journal is dense. **`JOURNAL_REGION_BYTES`** is **`JOURNAL_CAP_SLOTS * 5`**. Smaller journals mean checkpoints happen more often under heavy writes.

### `JOURNAL_CAP_SLOTS` and stable-memory compatibility

The header stores the configured slot count. **`StableRoaringBitmap::init`** requires it to equal this build’s **`JOURNAL_CAP_SLOTS`**. Otherwise opening fails with **`InitError::InvalidLayout`**.

**Binary builds compiled with different `JOURNAL_CAP_SLOTS` values are not drop-in compatible on the same stable-memory image:** journal length, **`JOURNAL_READ_CHUNK_BYTES`**, **`snapshot_base`**, and all subsequent offsets diverge.

To adopt a different capacity on a live canister, plan an explicit migration (for example replay logical operations into fresh storage laid out under the target build constant, export/import through your own serialization, or start from an empty bitmap on a cleared region)—this crate does not resize or relocate the layout in place.

### Choosing `JOURNAL_CAP_SLOTS` (default and tuning)

There is **no single optimal** slot count for every canister: behavior depends on how often you **mutate** the bitmap versus how often you **cold-open** it (for example after `init` on upgrade or reload) and on how full the journal tends to be before a checkpoint.

Rough guide:

- **Larger capacity** (for example **8192**) — Under steady **writes**, the journal fills less often, so **full-journal checkpoints occur less frequently** (each checkpoint is costly in snapshot size **S**). You pay a **larger fixed journal region** in stable memory (**`JOURNAL_REGION_BYTES`**) and, in paths that replay a **long** journal, **reopen / replay work can grow** with how much is recorded.
- **Smaller capacity** (for example **4096**) — **Smaller** stable layout and somewhat **lower worst-case reopen cost** when large journal backlogs matter, at the price of **more frequent** checkpoints when mutations keep the journal busy.

The **crate default** (**8192**) favors a common pattern—**mutations dominate, upgrades or full restarts are comparatively rare**—and reflects a judgment that **checkpoint thrashing** is often more painful than the extra journal bytes at this scale. That is **not** a universal fact: if you care more about **minimal stable footprint** or expect **`init`/replay** on a saturated journal often, benchmark or pick a **smaller** `JOURNAL_CAP_SLOTS` via the build-time env vars in **`build.rs`**. Migrating after launch always requires **layout-compatible** migration (see above).

## Usage notes

- Intended for **single-writer** use; do not alias the same stable memory through another API while an instance is live.
- Call **`StableRoaringBitmap::init`** whenever you need an instance (first boot with empty stable memory, after upgrade, or any reload). Empty memory is handled inside `init`; **`new`** is mainly for tests and code that wants **`BitmapError`** without mapping through **`InitError`**.
- Holding **`contains_view`** borrows the heap mirror; drop it before other operations on the same bitmap if you would mix reads and writes in one scope (see crate **Concurrency** docs).
- Logical length upper bound: **`JOURNAL_LEN_MAX`** = `u32::MAX + 1` (see crate docs).

## Example

```rust
use ic_stable_roaring::StableRoaringBitmap;
use ic_stable_structures::DefaultMemoryImpl;

let memory = DefaultMemoryImpl::default();
let bitset = StableRoaringBitmap::init(memory).unwrap();

bitset.insert(7).unwrap();
assert!(bitset.contains(7));

let memory = bitset.into_memory();
let bitset = StableRoaringBitmap::init(memory).unwrap();
assert!(bitset.contains(7));
```

## License

This project is licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT License](./LICENSE-MIT) at your option.
