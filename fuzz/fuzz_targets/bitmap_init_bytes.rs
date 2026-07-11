#![no_main]

use ic_stable_roaring::RoaringBitmap;
use ic_stable_structures::{Memory, vec_mem::VectorMemory};
use libfuzzer_sys::fuzz_target;
use std::panic::{AssertUnwindSafe, catch_unwind};

const MAX_IMAGE_BYTES: usize = 128 * 1024;
const WASM_PAGE_BYTES: usize = 65_536;

fuzz_target!(|input: &[u8]| {
    let image = &input[..input.len().min(MAX_IMAGE_BYTES)];
    let memory = VectorMemory::default();
    if !image.is_empty() {
        let pages = image.len().div_ceil(WASM_PAGE_BYTES) as u64;
        assert_ne!(memory.grow(pages), -1, "VectorMemory growth must succeed");
        memory.write(0, image);
    }

    let result = catch_unwind(AssertUnwindSafe(|| RoaringBitmap::init(memory)));
    let bitmap = result.expect("init must not panic");
    if let Ok(bitmap) = bitmap {
        let _ = bitmap.len();
        let _ = bitmap.is_empty();
        for index in [0, 1, 255, u16::MAX as u32, u32::MAX] {
            let _ = bitmap.contains(index);
        }
    }
});
