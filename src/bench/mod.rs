//! PocketIC / `canbench` harness for [`ic_stable_roaring::StableRoaringBitmap`].
//!
//! Cross-capacity comparisons: benchmarks whose names contain **`fixed`** hold their named workload
//! dimension (total writes, pending journal length, or snapshot size) **constant** regardless of
//! [`crate::JOURNAL_CAP_SLOTS`].
//! Fixed-pending **`reopen`** benches are **`cfg`‑gated in `build.rs`** (`journal_slots_gt_1024`,
//! `journal_slots_gt_4096`) when the crate is built without enough journal capacity.

use std::hint::black_box;

use crate::StableRoaringBitmap;
use canbench_rs::bench;
use ic_stable_structures::DefaultMemoryImpl;

mod wipe;

const INSERT_COUNT: u64 = 1_024;
const TRUNCATE_FROM: u64 = 2_048;
const TRUNCATE_TO: u64 = 1_024;
const LARGE_SNAPSHOT_BITS: u64 = 65_536;
const FIXED_CHECKPOINT_SNAPSHOT_BITS: u64 = 65_536;
const CONTAINS_BITMAP_BITS: u64 = 65_536;
const CONTAINS_QUERY_COUNT: u64 = 4_096;
const CONTAINS_QUERY_COUNT_LARGE: u64 = 32_768;
const CONTAINS_SPREAD_MULTIPLIER: u64 = 0x9E37;
const CONTAINS_SPREAD_INCREMENT: u64 = 0xB529;
const LARGE_TRUNCATE_FROM: u64 = 65_536;
const LARGE_TRUNCATE_TO: u64 = 32_768;
const JOURNAL_CAP_FILL: u64 = crate::JOURNAL_CAP_SLOTS as u64;
/// Largest journal prefix reachable through the public API before it checkpoints.
///
/// Capacities above one checkpoint before appending the final slot. Capacity one is special: every
/// append checkpoints first, then occupies its sole slot.
const JOURNAL_PREEMPTIVE_LIMIT: u64 = if JOURNAL_CAP_FILL == 1 {
    1
} else {
    JOURNAL_CAP_FILL - 1
};
const REPLAY_BLOCK: u64 = JOURNAL_CAP_FILL / 4;

/// Replay prefix length (floor of ~75% of journal capacity); every positive capacity is supported.
const REPLAY_THREE_QUARTERS: u64 = crate::JOURNAL_CAP_SLOTS as u64 * 3 / 4;

/// Small fixed prefix unrelated to journal cap — baseline reopen cost dominated by decode/scan start.
const JOURNAL_PREFIX_SMALL: u64 = 64;

/// How many roughly “full-journal equivalents” of sequential inserts to apply in sustained bench.
const JOURNAL_SUSTAINED_CYCLES: u64 = 8;

/// Sequential `insert` count **independent of journal capacity** (checkpoint cadence varies with cap).
const FIXED_TOTAL_SEQUENTIAL_INSERTS: u64 = 32_768;

#[cfg(journal_slots_gt_1024)]
const FIXED_JOURNAL_PENDING_1024: u64 = 1_024;

#[cfg(journal_slots_gt_4096)]
const FIXED_JOURNAL_PENDING_4096: u64 = 4_096;

fn make_bitset() -> StableRoaringBitmap<DefaultMemoryImpl> {
    StableRoaringBitmap::init(DefaultMemoryImpl::default()).expect("bitmap init")
}

fn populate(bitset: &StableRoaringBitmap<DefaultMemoryImpl>, count: u64) {
    for index in 0..count {
        bitset.insert(index as u32).expect("insert");
    }
}

/// Number of pending records after `count` state-changing appends with the preemptive checkpoint
/// rule. Callers use this only for capacities above one.
fn pending_records_after_appends(count: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let boundary = (crate::JOURNAL_CAP_SLOTS as u64).saturating_sub(1).max(1);
    1 + (count - 1) % boundary
}

fn bench_reopen_case(
    scope_name: &'static str,
    setup: impl FnOnce(&StableRoaringBitmap<DefaultMemoryImpl>),
) -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    setup(&bitset);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope(scope_name);
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        black_box((reopened.len(), reopened.contains(black_box(0u32))));
    })
}

fn setup_pure_replay_journal(bitset: &StableRoaringBitmap<DefaultMemoryImpl>) {
    let inserted = JOURNAL_PREEMPTIVE_LIMIT.div_ceil(2);
    for index in 0..inserted {
        bitset.insert(index as u32).expect("insert");
    }
    for index in 0..(JOURNAL_PREEMPTIVE_LIMIT - inserted) {
        bitset.clear(index as u32).expect("clear");
    }
}

fn setup_segmented_replay_journal(bitset: &StableRoaringBitmap<DefaultMemoryImpl>) {
    for index in 0..REPLAY_BLOCK {
        bitset.insert(index as u32).expect("insert");
    }
    for index in 0..REPLAY_BLOCK {
        bitset.clear(index as u32).expect("clear");
    }
    for index in 0..REPLAY_BLOCK {
        bitset.insert(index as u32).expect("insert");
    }
    bitset.ensure_len(REPLAY_BLOCK * 4).expect("ensure_len");
    bitset.truncate(REPLAY_BLOCK * 3).expect("truncate");
}

fn make_spread_queries(count: u64, modulo: u64) -> Vec<u32> {
    assert!(
        modulo.is_power_of_two(),
        "bitmap size should be a power of two"
    );
    assert!(modulo <= u32::MAX as u64 + 1);
    let mask = modulo - 1;
    let mut queries = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mixed = i
            .wrapping_mul(CONTAINS_SPREAD_MULTIPLIER)
            .wrapping_add(CONTAINS_SPREAD_INCREMENT);
        queries.push((mixed & mask) as u32);
    }
    queries
}

#[bench(raw)]
fn bench_roaring_insert_1024() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    canbench_rs::bench_fn(|| {
        let bitset = make_bitset();
        let _p = canbench_rs::bench_scope("roaring_insert");
        populate(&bitset, black_box(INSERT_COUNT));
        black_box(bitset.len());
    })
}

/// Measure one journal append and one heap mutation after setup has been excluded from the scope.
#[bench(raw)]
fn bench_roaring_insert_single_append() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, INSERT_COUNT - 1);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_insert_single_append");
        bitset
            .insert(black_box((INSERT_COUNT - 1) as u32))
            .expect("insert");
        black_box(bitset.len());
    })
}

/// Repeatedly toggle one bit to keep Roaring container shape stable while exercising journaling.
#[bench(raw)]
fn bench_roaring_set_toggle_journal() -> canbench_rs::BenchResult {
    if JOURNAL_PREEMPTIVE_LIMIT == 0 {
        return canbench_rs::BenchResult::default();
    }

    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    bitset.set(0, false).expect("initialize toggle bit");
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_set_toggle_journal");
        let mut value = false;
        for _ in 0..JOURNAL_PREEMPTIVE_LIMIT {
            value = !value;
            bitset.set(0, black_box(value)).expect("toggle bit");
        }
        black_box((bitset.len(), value));
    })
}

#[bench(raw)]
fn bench_roaring_contains_1024() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, INSERT_COUNT);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_contains");
        let index = black_box(INSERT_COUNT - 1);
        black_box(bitset.contains(index as u32));
    })
}

#[bench(raw)]
fn bench_roaring_contains_65536_4096() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, CONTAINS_BITMAP_BITS);
    let queries = make_spread_queries(CONTAINS_QUERY_COUNT, CONTAINS_BITMAP_BITS);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_contains_large");
        let mut acc = false;
        for index in black_box(&queries) {
            acc ^= bitset.contains(*index);
        }
        black_box(acc);
    })
}

#[bench(raw)]
fn bench_roaring_contains_65536_32768() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, CONTAINS_BITMAP_BITS);
    let queries = make_spread_queries(CONTAINS_QUERY_COUNT_LARGE, CONTAINS_BITMAP_BITS);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_contains_large_32768");
        let mut acc = false;
        for index in black_box(&queries) {
            acc ^= bitset.contains(*index);
        }
        black_box(acc);
    })
}

#[bench(raw)]
fn bench_roaring_contains_view_65536_32768() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, CONTAINS_BITMAP_BITS);
    let queries = make_spread_queries(CONTAINS_QUERY_COUNT_LARGE, CONTAINS_BITMAP_BITS);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_contains_view_large_32768");
        let view = bitset.contains_view();
        let mut acc = false;
        for index in black_box(&queries) {
            acc ^= view.contains(*index);
        }
        black_box(acc);
    })
}

#[bench(raw)]
fn bench_roaring_truncate_2048_to_1024() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, TRUNCATE_FROM);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_truncate");
        bitset.truncate(black_box(TRUNCATE_TO)).expect("truncate");
        black_box(bitset.len());
    })
}

#[bench(raw)]
fn bench_roaring_reopen_after_preemptive_checkpoint() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, JOURNAL_CAP_FILL);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_after_checkpoint");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        black_box(reopened.contains(black_box((JOURNAL_CAP_FILL - 1) as u32)));
    })
}

#[bench(raw)]
fn bench_roaring_reopen_after_pure_journal_full() -> canbench_rs::BenchResult {
    bench_reopen_case("roaring_reopen_pure_journal", setup_pure_replay_journal)
}

#[bench(raw)]
fn bench_roaring_reopen_after_segmented_journal_full() -> canbench_rs::BenchResult {
    bench_reopen_case(
        "roaring_reopen_segmented_journal",
        setup_segmented_replay_journal,
    )
}

#[bench(raw)]
fn bench_roaring_truncate_large_suffix_65536_to_32768() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, LARGE_TRUNCATE_FROM);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_truncate_large");
        bitset
            .truncate(black_box(LARGE_TRUNCATE_TO))
            .expect("truncate");
        black_box(bitset.len());
    })
}

#[bench(raw)]
fn bench_roaring_checkpoint_after_full_journal() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    // `append_record` checkpoints before consuming the final slot. Fill exactly up to that
    // boundary so the measured insert triggers the checkpoint rather than merely appending.
    populate(&bitset, JOURNAL_CAP_FILL.saturating_sub(1));
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_checkpoint");
        bitset
            .insert(black_box(JOURNAL_CAP_FILL.saturating_sub(1) as u32))
            .expect("insert triggering checkpoint");
        black_box(bitset.contains(black_box(JOURNAL_CAP_FILL.saturating_sub(1) as u32)));
    })
}

/// Checkpoint a fixed-size snapshot after filling the journal boundary with reversible mutations.
/// Unlike `bench_roaring_checkpoint_after_full_journal`, the snapshot size does not vary with
/// `JOURNAL_CAP_SLOTS`, so this isolates capacity's effect on checkpoint scheduling and clearing.
#[bench(raw)]
fn bench_roaring_checkpoint_fixed_snapshot_65536() -> canbench_rs::BenchResult {
    if crate::JOURNAL_CAP_SLOTS == 1 {
        return canbench_rs::BenchResult::default();
    }

    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, FIXED_CHECKPOINT_SNAPSHOT_BITS);

    let pending = pending_records_after_appends(FIXED_CHECKPOINT_SNAPSHOT_BITS);
    let mut bit_zero_is_set = true;
    for _ in 0..(crate::JOURNAL_CAP_SLOTS as u64 - 1 - pending) {
        bit_zero_is_set = !bit_zero_is_set;
        bitset
            .set(0, bit_zero_is_set)
            .expect("reversible journal fill");
    }

    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_checkpoint_fixed_snapshot_65536");
        bitset
            .set(0, !bit_zero_is_set)
            .expect("set triggering checkpoint");
        black_box(bitset.contains(black_box(0)));
    })
}

#[bench(raw)]
fn bench_roaring_reopen_after_large_snapshot_65536() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, LARGE_SNAPSHOT_BITS);
    bitset
        .insert(LARGE_SNAPSHOT_BITS as u32)
        .expect("insert triggering checkpoint");
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_large");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        black_box(reopened.contains(black_box(LARGE_SNAPSHOT_BITS as u32)));
    })
}

/// Reopen while the journal holds the largest pending prefix the public API can produce before a
/// preemptive checkpoint. Journal scan + replay cost scales with `JOURNAL_CAP_SLOTS`.
#[bench(raw)]
fn bench_roaring_reopen_journal_at_preemptive_limit() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, JOURNAL_PREEMPTIVE_LIMIT);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_journal_at_preemptive_limit");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        let last = black_box((JOURNAL_PREEMPTIVE_LIMIT - 1) as u32);
        black_box((reopened.len(), reopened.contains(last)));
    })
}

/// Replay a long but partial journal — stable read + apply work grows with `REPLAY_THREE_QUARTERS`,
/// which scales with `JOURNAL_CAP_SLOTS`.
#[bench(raw)]
fn bench_roaring_reopen_journal_three_quarters() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, REPLAY_THREE_QUARTERS);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_journal_three_quarters");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        let last = black_box((REPLAY_THREE_QUARTERS.saturating_sub(1)) as u32);
        black_box((reopened.len(), reopened.contains(last)));
    })
}

/// Small journal prefix for comparison with `bench_roaring_reopen_journal_*` (should change little
/// when only `JOURNAL_CAP_SLOTS` changes, since replay stops at the first zero record).
#[bench(raw)]
fn bench_roaring_reopen_journal_prefix_small() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, JOURNAL_PREFIX_SMALL);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_journal_prefix_small");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        let last = black_box((JOURNAL_PREFIX_SMALL.saturating_sub(1)) as u32);
        black_box((reopened.len(), reopened.contains(last)));
    })
}

/// Repeatedly fill the journal and checkpoint by streaming sequential inserts. Total journal bytes
/// appended and checkpoint frequency both move with `JOURNAL_CAP_SLOTS`.
#[bench(raw)]
fn bench_roaring_sequential_inserts_sustained_journal() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    canbench_rs::bench_fn(|| {
        wipe::wipe_stable_memory();
        let bitset = make_bitset();
        let cap = crate::JOURNAL_CAP_SLOTS as u64;
        let total_indices = black_box(cap.saturating_mul(JOURNAL_SUSTAINED_CYCLES));
        let _p = canbench_rs::bench_scope("roaring_seq_inserts_journal");
        for i in 0..total_indices {
            bitset.insert(i as u32).expect("sequential insert");
        }
        let last = (total_indices.saturating_sub(1)) as u32;
        black_box((bitset.len(), bitset.contains(black_box(last))));
    })
}

/// Stream **`32768`** sequential `insert`s — workload **does not scale** with [`crate::JOURNAL_CAP_SLOTS`];
/// checkpoint counts and snapshot sizes still depend on capacity.
#[bench(raw)]
fn bench_roaring_sequential_inserts_fixed_32768() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    canbench_rs::bench_fn(|| {
        wipe::wipe_stable_memory();
        let bitset = make_bitset();
        let n = black_box(FIXED_TOTAL_SEQUENTIAL_INSERTS);
        let _p = canbench_rs::bench_scope("roaring_seq_inserts_fixed_32768");
        for i in 0..n {
            bitset.insert(i as u32).expect("fixed sequential insert");
        }
        let last = (n.saturating_sub(1)) as u32;
        black_box((bitset.len(), bitset.contains(black_box(last))));
    })
}

/// Reopen with **exactly 1024** pending `insert` journal records (`0..1024`).
///
/// Compiled only when **`JOURNAL_CAP_SLOTS > 1024`** so all 1024 records remain pending without a
/// preemptive checkpoint (`build.rs` emits `journal_slots_gt_1024`).
#[cfg(journal_slots_gt_1024)]
#[bench(raw)]
fn bench_roaring_reopen_journal_fixed_pending_1024() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, FIXED_JOURNAL_PENDING_1024);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_journal_fixed_1024");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        black_box(reopened.contains(black_box(1023u32)));
    })
}

/// Reopen with **4096** pending inserts (indices `0..4095`).
///
/// Compiled only when **`JOURNAL_CAP_SLOTS > 4096`** so all records remain pending without a
/// preemptive checkpoint (`build.rs` emits `journal_slots_gt_4096`).
#[cfg(journal_slots_gt_4096)]
#[bench(raw)]
fn bench_roaring_reopen_journal_fixed_pending_4096() -> canbench_rs::BenchResult {
    wipe::wipe_stable_memory();
    let bitset = make_bitset();
    populate(&bitset, FIXED_JOURNAL_PENDING_4096);
    canbench_rs::bench_fn(|| {
        let _p = canbench_rs::bench_scope("roaring_reopen_journal_fixed_4096");
        let reopened = StableRoaringBitmap::init(bitset.into_memory()).expect("reopen");
        black_box(reopened.contains(black_box(4095u32)));
    })
}
