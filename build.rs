//! Writes **`journal_layout.rs`** under Cargo's `OUT_DIR`.
//!
//! Optional env vars (compile time):
//! - **`JOURNAL_CAP_SLOTS`** — journal slot capacity (default **`6144`**; any positive **`usize`**
//!   whose fixed layout addresses fit in `u64`).
//!   The default (**6144**) balances **steady mutation** workloads (fewer checkpoints than **`4096`**) against
//!   a smaller stable journal than **`8192`** and somewhat lighter long-journal replay — see README **Choosing
//!   `JOURNAL_CAP_SLOTS`**. Larger or smaller caps are compile-time overrides via this env var.
//!   This value is stored in the stable-memory header; journal size and the rest of the layout derive
//!   from it, so **Wasm modules compiled with different caps are not interchangeable** on the same
//!   backing memory unless you migrate externally (see the `ic_stable_roaring` crate README and crate
//!   root docs).
//! - **`JOURNAL_READ_CHUNK_MAX`** — optional upper bound on chunk / stack buffer (default **`32768`**;
//!   cannot exceed the non-overridable **`32768`** byte ceiling).
//! - **`JOURNAL_READ_CHUNK_TARGET`** — **preferred** upper bound on chunk (default **`5120`**). The
//!   build picks the **largest** valid chunk **under this cap first**; it must be at least **`5`**.
//!
//! **Selection rule:** let **`R = JOURNAL_CAP_SLOTS * 5`**. `JOURNAL_READ_CHUNK_BYTES` is the **largest**
//! **`d`** such that **`d % 5 == 0`**, **`R % d == 0`**, and **`d <= floor5(min(R, JOURNAL_READ_CHUNK_TARGET, JOURNAL_READ_CHUNK_MAX))`**
//! (`floor5` rounds down to a multiple of **`5`**). For valid inputs there is always a solution (**`d = 5`** works).
//!
//! Rationale: `init` always performs at least one full `Memory::read` of size **`JOURNAL_READ_CHUNK_BYTES`**
//! before it can know the journal is empty. Capping the **preferred** chunk near the historical
//! **`5120`** avoids **read amplification** on checkpointed (empty-tail) reopen paths. Users may
//! raise **`JOURNAL_READ_CHUNK_MAX`** only up to the fixed stack / buffer ceiling.

/// Default preferred replay read size (`Memory::read` / `[u8; N]` in `bitmap::RoaringBitmap::init`).
/// Picked to match historical behavior when it divides **`JOURNAL_CAP_SLOTS * 5`**.
const DEFAULT_CHUNK_TARGET: usize = 5120;

/// Non-overridable upper bound on `JOURNAL_READ_CHUNK_BYTES` (Wasm stack `[u8; N]`).
///
/// Keeping this fixed also bounds the divisor search below to at most `32 KiB / 5` candidates.
const ABSOLUTE_CHUNK_HARD_MAX: usize = 32 * 1024;
const HEADER_SIZE: u64 = 64;
const JOURNAL_RECORD_SIZE: u64 = 5;

#[inline]
fn floor_multiple_of_five(n: usize) -> usize {
    (n / 5) * 5
}

/// Largest **`chunk`** with **`chunk % 5 == 0`**, **`region % chunk == 0`**, **`5 <= chunk <= floor5(min(cap, region))`**.
fn greatest_valid_chunk_bounded(region: usize, cap: usize) -> Option<usize> {
    assert_eq!(region % 5, 0);
    assert!(region >= 5);

    let cap = floor_multiple_of_five(cap.min(region));
    if cap < 5 {
        return None;
    }

    let mut chunk = cap;
    loop {
        if region.is_multiple_of(chunk) {
            return Some(chunk);
        }
        if chunk < 5 {
            return None;
        }
        chunk -= 5;
    }
}

fn write_if_changed(path: &std::path::Path, contents: &str) {
    if std::fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return;
    }
    std::fs::write(path, contents).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn choose_journal_read_chunk(region: usize, target: usize, hard_max: usize) -> usize {
    assert!(
        hard_max >= 5,
        "JOURNAL_READ_CHUNK_MAX ({hard_max}) must be >= 5 (journal record size)"
    );

    let target_cap = floor_multiple_of_five(target.min(hard_max).min(region));
    assert!(
        target_cap >= 5,
        "JOURNAL_READ_CHUNK_TARGET ({target}) is too small after rounding; need at least one 5-byte journal record"
    );

    greatest_valid_chunk_bounded(region, target_cap).unwrap_or_else(|| {
        panic!("internal error: chunk=5 should divide region={region}; target_cap={target_cap}")
    })
}

fn main() {
    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR must be set by Cargo");
    let out_path = std::path::Path::new(&out_dir).join("journal_layout.rs");

    let slots_str = std::env::var("JOURNAL_CAP_SLOTS").unwrap_or_else(|_| "6144".to_string());
    let slots: usize = slots_str.parse().unwrap_or_else(|err| {
        panic!("JOURNAL_CAP_SLOTS={slots_str:?} is not a valid usize: {err}");
    });
    assert!(slots > 0, "JOURNAL_CAP_SLOTS must be > 0");

    let max_chunk_str = std::env::var("JOURNAL_READ_CHUNK_MAX")
        .unwrap_or_else(|_| ABSOLUTE_CHUNK_HARD_MAX.to_string());
    let max_chunk: usize = max_chunk_str.parse().unwrap_or_else(|err| {
        panic!("JOURNAL_READ_CHUNK_MAX={max_chunk_str:?} is not a valid usize: {err}")
    });
    assert!(
        max_chunk >= 5,
        "JOURNAL_READ_CHUNK_MAX ({max_chunk}) must be >= 5 (journal record size)"
    );
    assert!(
        max_chunk <= ABSOLUTE_CHUNK_HARD_MAX,
        "JOURNAL_READ_CHUNK_MAX ({max_chunk}) exceeds the non-overridable stack limit ({ABSOLUTE_CHUNK_HARD_MAX})"
    );

    let chunk_target_str = std::env::var("JOURNAL_READ_CHUNK_TARGET")
        .unwrap_or_else(|_| DEFAULT_CHUNK_TARGET.to_string());
    let chunk_target: usize = chunk_target_str.parse().unwrap_or_else(|err| {
        panic!("JOURNAL_READ_CHUNK_TARGET={chunk_target_str:?} is not a valid usize: {err}")
    });
    assert!(
        chunk_target >= 5,
        "JOURNAL_READ_CHUNK_TARGET ({chunk_target}) must be >= 5"
    );

    let region = slots
        .checked_mul(5)
        .expect("JOURNAL_CAP_SLOTS * 5 must fit in usize");
    assert!(
        region >= 5,
        "journal region ({region} bytes): internal invariant violated (need >= one record)"
    );
    let journal_end = HEADER_SIZE
        .checked_add(
            (slots as u64)
                .checked_mul(JOURNAL_RECORD_SIZE)
                .expect("JOURNAL_CAP_SLOTS * journal record size must fit in u64"),
        )
        .expect("journal end address must fit in u64");
    let snapshot_base = journal_end
        .checked_add(7)
        .expect("aligned snapshot base must fit in u64")
        & !7;

    let read_chunk = choose_journal_read_chunk(region, chunk_target, max_chunk);

    let contents = format!(
        "// Generated by build.rs (`JOURNAL_CAP_SLOTS`, `JOURNAL_READ_CHUNK_*`).\n\
         // Cargo creates this file in OUT_DIR for the active build configuration.\n\
         \n\
         /// Journal slot capacity set at crate build time; must match header offset `12` (`u64`) on disk.\n\
         pub const JOURNAL_CAP_SLOTS: usize = {slots};\n\
         \n\
         /// Byte offset immediately after the fixed journal region, validated at build time.\n\
         pub const JOURNAL_END_BYTES: u64 = {journal_end};\n\
         \n\
         /// Eight-byte-aligned snapshot start offset, validated at build time.\n\
         pub const JOURNAL_SNAPSHOT_BASE: u64 = {snapshot_base};\n\
         \n\
         /// Replay read granularity: greatest divisor of `JOURNAL_CAP_SLOTS * 5` under `JOURNAL_READ_CHUNK_TARGET`,\n\
         /// capped by `JOURNAL_READ_CHUNK_MAX` (multiples of `5` only).\n\
         pub const JOURNAL_READ_CHUNK_BYTES: usize = {read_chunk};\n"
    );
    write_if_changed(&out_path, &contents);

    // Gate `#[cfg]` in `bench` so fixed-journal benchmarks are omitted when compile-time caps are too small.
    println!("cargo:rustc-check-cfg=cfg(journal_slots_ge_1024)");
    println!("cargo:rustc-check-cfg=cfg(journal_slots_ge_4096)");
    if slots >= 1024 {
        println!("cargo:rustc-cfg=journal_slots_ge_1024");
    }
    if slots >= 4096 {
        println!("cargo:rustc-cfg=journal_slots_ge_4096");
    }

    println!("cargo:rerun-if-env-changed=JOURNAL_CAP_SLOTS");
    println!("cargo:rerun-if-env-changed=JOURNAL_READ_CHUNK_MAX");
    println!("cargo:rerun-if-env-changed=JOURNAL_READ_CHUNK_TARGET");
}
