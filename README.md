# ic-stable-roaring

Stable-memory [roaring bitmap](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html) for [Internet Computer](https://internetcomputer.org/) canisters. Reads use a heap mirror; `set`, `insert`, `clear`, `ensure_len`, and `truncate` persist through an append-only journal and occasional snapshots into stable memory.

## Documentation

| Location                                                         | What to read there                                                                                                    |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| **This README**                                                  | What the crate does, how to get started, operational caveats.                                                         |
| **Crate docs** (`cargo doc --open`, then `ic_stable_roaring`)    | On-disk **layout**, journal packing, **constants** (`JOURNAL_CAP_SLOTS`, `JOURNAL_LEN_MAX`, …), and crate-wide rules. |
| **Type & method docs** (`StableRoaringBitmap` / `RoaringBitmap`) | Durability, **checkpointing**, **per-method time bounds**, **`init` as the normal constructor**, and error types.     |

Complexity and edge cases (for example checkpoint cost **Θ(S)** when the journal fills, or **O(C)** work when truncating a suffix) live on those API docs—see **Time complexity** on each method.

## Operations (overview)

- **`contains`** — heap mirror only; out-of-range indices are `false`.
- **`set` / `insert` / `clear`** — journal + heap update **when the logical bit changes** (no journal record on a no-op); see the `set` docs for rare checkpoint behavior even on idempotent sets.
- **`ensure_len`** — extends the exclusive logical length without materializing zero bits (capped by `JOURNAL_LEN_MAX`).
- **`truncate`** — shortens the logical length and clears set bits from the new end onward (same cap).
- **`len` / `is_empty`** — **logical** length, not set-bit cardinality.

## Usage notes

- Intended for **single-writer** use; do not alias the same stable memory through another API while an instance is live.
- Call **`StableRoaringBitmap::init`** whenever you need an instance (first boot with empty stable memory, after upgrade, or any reload). Empty memory is handled inside `init`; you should not need **`new`** in application code (it exists for tests and `GrowFailed`-typed bootstrap).
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
