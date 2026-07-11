# ic-stable-roaring

Stable-memory [Roaring Bitmap](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html) for [Internet Computer](https://internetcomputer.org/) canisters. Reads use a heap mirror; `set`, `insert`, `clear`, `ensure_len`, and `truncate` persist through an append-only journal and occasional snapshots into stable memory.

**Durability:** On IC, a canister call runs to completion or traps; after a mutating method returns `Ok`, its stable writes for that call have finished. This crate does not add a separate crash-recovery protocol beyond validating bytes on `init` (`InvalidLayout` if the snapshot is inconsistent). See **`cargo doc`** on the `ic-stable-roaring` crate for the full **Durability** section.

## Documentation

| Location                                                         | What to read there                                                                                                                                                                           |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **This README**                                                  | What the crate does, how to get started, operational caveats.                                                                                                                                |
| **Crate docs** (`cargo doc --open`, then `ic_stable_roaring`)    | On-disk **layout**, journal packing, **constants** (`JOURNAL_CAP_SLOTS` / `JOURNAL_READ_CHUNK_TARGET` / `JOURNAL_READ_CHUNK_MAX` in `build.rs`, `JOURNAL_LEN_MAX`, …), and crate-wide rules. |
| **Type & method docs** (`StableRoaringBitmap` / `RoaringBitmap`) | Durability, **checkpointing**, **per-method time bounds**, **`init` as the normal constructor**, and errors (`BitmapError`, `InitError`).                                                    |

Complexity and edge cases (for example checkpoint cost **Θ(S)** before the journal's final slot would be consumed, or **O(C)** work when truncating a suffix) live on those API docs—see **Time complexity** on each method.

## Operations (overview)

- **`contains`** — heap mirror only; out-of-range indices are `false`.
- **`set` / `insert` / `clear`** — journal + heap update **when the logical bit changes** (no journal record on a no-op); see the `set` docs for rare checkpoint behavior even on idempotent sets.
- **`ensure_len`** — extends the exclusive logical length without materializing zero bits (capped by `JOURNAL_LEN_MAX`).
- **`truncate`** — shortens the logical length and clears set bits from the new end onward (same cap).
- **`len` / `is_empty`** — **logical** length, not set-bit cardinality.

Mutations return **`Result<..., BitmapError>`** (`LimitsExceeded`, `GrowFailed`, snapshot **I/O**). Lengths above **`JOURNAL_LEN_MAX`** are reported as `LimitsExceeded`, not panics.

The on-disk journal records **logical state changes**, not every redundant `set` call—do not treat it as a verbatim audit log. Journal capacity is fixed when the crate is built (**`JOURNAL_CAP_SLOTS`**, default `6144`). At compile time, set **`JOURNAL_CAP_SLOTS`** to a positive **`usize`** whose fixed layout offsets fit in `u64`; invalid extreme values fail the build. **`JOURNAL_READ_CHUNK_BYTES`** is chosen in **`build.rs`** as the **largest divisor** of **`JOURNAL_CAP_SLOTS * 5`** that is a multiple of **`5`** and does not exceed **`floor5(min(JOURNAL_CAP_SLOTS * 5, JOURNAL_READ_CHUNK_TARGET, JOURNAL_READ_CHUNK_MAX))`** (defaults: target **`5120`**, max **`32768`**). `init` replays the contiguous nonzero prefix and stops at the first all-zero record. It does not scan or reject nonzero bytes later in the unused journal region; as with `stable-structures`, callers must keep this stable-memory region isolated and valid. The chunk target controls the granularity of replay reads: recovery reads through the chunk containing the first empty record, rather than the full fixed region. **`JOURNAL_READ_CHUNK_MAX`** may lower the limit but cannot exceed the non-overridable **32768-byte** Wasm stack / buffer ceiling. Set **`JOURNAL_READ_CHUNK_TARGET`** high (up to the effective max) if you intentionally want fewer, larger reads when the journal is dense. **`JOURNAL_REGION_BYTES`** is **`JOURNAL_CAP_SLOTS * 5`**. Smaller journals mean checkpoints happen more often under heavy writes.

### `JOURNAL_CAP_SLOTS` and stable-memory compatibility

The header stores the configured slot count. **`StableRoaringBitmap::init`** requires it to equal this build’s **`JOURNAL_CAP_SLOTS`**. Otherwise opening fails with **`InitError::InvalidLayout`**.

**Binary builds compiled with different `JOURNAL_CAP_SLOTS` values are not drop-in compatible on the same stable-memory image:** journal length, **`JOURNAL_READ_CHUNK_BYTES`**, **`snapshot_base`**, and all subsequent offsets diverge.

To adopt a different capacity on a live canister, plan an explicit migration (for example replay logical operations into fresh storage laid out under the target build constant, export/import through your own serialization, or start from an empty bitmap on a cleared region)—this crate does not resize or relocate the layout in place.

### Choosing `JOURNAL_CAP_SLOTS` (default and tuning)

There is **no single optimal** slot count for every canister: behavior depends on how often you **mutate** the bitmap versus how often you **cold-open** it (for example after `init` on upgrade or reload) and on how full the journal tends to be before a checkpoint.

Rough guide:

- **Larger capacity** (for example **8192**) — Under steady **writes**, the journal approaches its final slot less often, so **checkpoints occur less frequently** (each checkpoint is costly in snapshot size **S**). You pay a **larger fixed journal region** in stable memory (**`JOURNAL_REGION_BYTES`**) and, in paths that replay a **long** journal, **reopen / replay work can grow** with how much is recorded.
- **Smaller capacity** (for example **4096**) — **Smaller** stable layout and somewhat **lower worst-case reopen cost** when large journal backlogs matter, at the price of **more frequent** checkpoints when mutations keep the journal busy.

The **crate default** (**6144**) is a **middle ground** between **`4096`** and **`8192`**: enough headroom to reduce checkpoint pressure versus **`4096`**, with a **`JOURNAL_REGION_BYTES`** of **`6144 × 5`** (30 KiB) instead of **`8192 × 5`** so long-journal **`init`/replay** stays cheaper than under the larger cap under similar backlog. Tune **down** (**4096**) for minimal stable footprint and reopen-heavy patterns, **up** (**8192** or beyond, still `usize`-limited) under very heavy steady writes—the trade-off stays workload-dependent (**`build.rs`** env vars). Migrating after launch always requires **layout-compatible** migration (see above).

### Recovery integrity boundary

`init` intentionally trusts a valid, isolated stable-memory region and stops replay at the first empty journal slot. This matches `stable-structures` production initialization, which validates headers and reads reachable data rather than scanning unused allocation space. A committed-length-header experiment also avoided the tail scan, but its extra stable header write regressed `insert_1024` by about 9% and sustained inserts by about 13–14%, so it was rejected. If an application needs to detect arbitrary out-of-band corruption of the unused tail, it must provide an explicit offline integrity audit; normal recovery does not pay that cost. Measure capacity trade-offs in the target PocketIC environment with:

```sh
JOURNAL_CAP_SWEEP_SLOTS='1024 2048 4096 6144' \
  ./scripts/sweep_journal_cap_canbench.sh \
  bench_roaring_reopen_journal_prefix_small \
  bench_roaring_reopen_journal_fixed_pending_1024 \
  bench_roaring_checkpoint_after_full_journal \
  bench_roaring_checkpoint_fixed_snapshot_65536 \
  bench_roaring_sequential_inserts_fixed_32768
```

`bench_roaring_reopen_journal_fixed_pending_1024` is intentionally omitted when capacity is exactly `1024`: the final pending record would trigger the checkpoint being avoided by that benchmark.

### `roaring` dependency compatibility

The stable-memory header version covers this crate's header and journal layout, **not** the
`roaring` crate version. Compatible upstream reader updates must keep opening the committed
standard-Roaring `roaring` 0.11.4 fixture (`tests/fixtures/`); the fixture test checks semantic
membership rather than expecting new writers to emit byte-identical output. Bump the stable header
version only when this crate changes its own layout or journal encoding.

## Usage notes

- Intended for **single-writer** use; do not alias the same stable memory through another API while an instance is live.
- Call **`StableRoaringBitmap::init`** whenever you need an instance (first boot with empty stable memory, after upgrade, or any reload). Empty memory is handled inside `init`; **`new`** is mainly for tests and code that wants **`BitmapError`** without mapping through **`InitError`**.
- Holding **`contains_view`** borrows the heap mirror; drop it before other operations on the same bitmap if you would mix reads and writes in one scope (see crate **Concurrency** docs).
- Logical length upper bound: **`JOURNAL_LEN_MAX`** = `u32::MAX + 1` (see crate docs).

## Development validation

Run `scripts/test_layout_matrix.sh` to compile and test the default layout plus small-capacity and
chunk-selection boundary configurations in isolated temporary target directories.

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
