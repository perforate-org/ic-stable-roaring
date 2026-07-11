#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use ic_stable_roaring::RoaringBitmap;
use ic_stable_structures::vec_mem::VectorMemory;
use libfuzzer_sys::fuzz_target;
use std::collections::BTreeSet;

const MAX_OPERATIONS: usize = 128;
const INDEX_DOMAIN: u32 = 255;

#[derive(Arbitrary, Debug)]
enum Operation {
    Insert(u16),
    Clear(u16),
    EnsureLen(u16),
    Truncate(u16),
    Reopen,
}

fn index(value: u16) -> u32 {
    u32::from(value) % (INDEX_DOMAIN + 1)
}

fn assert_matches_oracle(
    bitmap: &RoaringBitmap<VectorMemory>,
    expected_len: u64,
    expected_set: &BTreeSet<u32>,
) {
    assert_eq!(bitmap.len(), expected_len);
    for candidate in 0..=INDEX_DOMAIN {
        assert_eq!(
            bitmap.contains(candidate),
            expected_set.contains(&candidate),
            "membership mismatch at {candidate}"
        );
    }
}

fuzz_target!(|input: &[u8]| {
    let mut input = Unstructured::new(input);
    let mut bitmap = RoaringBitmap::new(VectorMemory::default()).expect("fresh bitmap");
    let mut expected_len = 0;
    let mut expected_set = BTreeSet::new();

    for step in 0..MAX_OPERATIONS {
        let operation = match Operation::arbitrary(&mut input) {
            Ok(operation) => operation,
            Err(_) => break,
        };
        match operation {
            Operation::Insert(value) => {
                let index = index(value);
                bitmap.insert(index).expect("bounded insert");
                expected_set.insert(index);
                expected_len = expected_len.max(u64::from(index) + 1);
            }
            Operation::Clear(value) => {
                let index = index(value);
                bitmap.clear(index).expect("bounded clear");
                expected_set.remove(&index);
            }
            Operation::EnsureLen(value) => {
                let len = u64::from(index(value));
                bitmap.ensure_len(len).expect("bounded ensure_len");
                expected_len = expected_len.max(len);
            }
            Operation::Truncate(value) => {
                let len = u64::from(index(value));
                bitmap.truncate(len).expect("bounded truncate");
                if len < expected_len {
                    expected_len = len;
                    expected_set.retain(|index| u64::from(*index) < len);
                }
            }
            Operation::Reopen => {
                bitmap = RoaringBitmap::init(bitmap.into_memory()).expect("reopen bitmap");
            }
        }
        if (step + 1).is_multiple_of(16) {
            bitmap = RoaringBitmap::init(bitmap.into_memory()).expect("periodic reopen bitmap");
        }
        assert_matches_oracle(&bitmap, expected_len, &expected_set);
    }

    bitmap = RoaringBitmap::init(bitmap.into_memory()).expect("final reopen");
    assert_matches_oracle(&bitmap, expected_len, &expected_set);
});
