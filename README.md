# ic-stable-roaring

Persistent [Roaring Bitmap](https://docs.rs/roaring/latest/roaring/bitmap/struct.RoaringBitmap.html)
for [Internet Computer](https://internetcomputer.org/) canisters. It stores bitmap state in stable
memory so it survives upgrades without application-level serialization hooks.

## How it works

Reads use a heap mirror. Logical mutations are appended to a stable-memory journal and periodically
checkpointed as a Roaring snapshot. Calling `init` restores the snapshot and replays pending journal
records.

## Example

Each bitmap must exclusively own its stable memory. Use `MemoryManager` when a canister has multiple
stable structures.

```rust
use ic_stable_roaring::StableRoaringBitmap;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager},
    DefaultMemoryImpl,
};

let memory_manager = MemoryManager::init(DefaultMemoryImpl::default());
let bitmap = StableRoaringBitmap::init(memory_manager.get(MemoryId::new(0))).unwrap();

bitmap.insert(42).unwrap();
assert!(bitmap.contains(42));
```

## Usage

- Use `StableRoaringBitmap::init` on first boot and after every upgrade or reload.
- `insert`, `clear`, `ensure_len`, and `truncate` return `Result`; `contains` reads the heap mirror.
- `len` is the exclusive logical bit length, not the number of set bits.
- Do not mutate the same `Memory` through another structure while a bitmap uses it.

## Storage configuration

The default `JOURNAL_CAP_SLOTS` is **4096** (a 20 KiB journal). It balances writes with bounded
worst-case reopen work. Raise it only for a write-heavy workload that can accept slower recovery of
a long pending journal.

The capacity is stored in the stable header. Builds with different capacities are not compatible on
the same stable-memory image; migrate into fresh storage before changing it in a live canister.

`init` expects valid, isolated stable memory. It validates the reachable header, snapshot, and
journal prefix, then stops at the first empty journal record.

## Documentation

Run `cargo doc --open` for API contracts, error types, stable layout, durability, and per-method
complexity. See [CONTRIBUTING.md](./CONTRIBUTING.md) for development checks and layout-change rules.

## Contributing

Issues and pull requests are welcome at [GitHub](https://github.com/perforate-org/ic-stable-roaring).
Please read [CONTRIBUTING.md](./CONTRIBUTING.md) and [SECURITY.md](./SECURITY.md) first.

## License

Licensed under either [Apache License, Version 2.0](./LICENSE-APACHE) or
[MIT License](./LICENSE-MIT), at your option.
