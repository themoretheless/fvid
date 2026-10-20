//! Standalone optimized benchmark of the actual CAS, geometry and parser source.
//! rustc --edition=2024 -O docs/improvements/benchmark-cas-unselected.rs -o /tmp/fvid-cas-probe
#![allow(dead_code)]
#[path = "../../crates/fvid-media/src/owned_cas.rs"]
mod owned_cas;
#[path = "../../crates/fvid-media/src/owned_expression.rs"]
mod owned_expression;
#[path = "../../crates/fvid-media/src/owned_frame.rs"]
mod owned_frame;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};
struct Tracked;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Tracked {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
}
#[global_allocator]
static GLOBAL: Tracked = Tracked;
fn main() {
    for (width, height, depth, calls) in [
        (1920, 1080, 8, 128),
        (3840, 2160, 8, 32),
        (1920, 1080, 16, 64),
    ] {
        let mut data = vec![127; width * height * 3 * if depth == 8 { 1 } else { 2 }];
        let filter = owned_cas::Cas::parse("strength=0.8:planes=8").unwrap();
        for _ in 0..4 {
            filter
                .apply_rgb(black_box(&mut data), width, height, depth, 3)
                .unwrap();
        }
        ALLOCS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        TRACK.store(true, Ordering::Relaxed);
        let start = Instant::now();
        for _ in 0..calls {
            filter
                .apply_rgb(
                    black_box(&mut data),
                    black_box(width),
                    black_box(height),
                    black_box(depth),
                    black_box(3),
                )
                .unwrap();
        }
        let elapsed = start.elapsed();
        TRACK.store(false, Ordering::Relaxed);
        assert!(data.iter().all(|&v| v == 127));
        println!(
            "{{\"width\":{width},\"height\":{height},\"depth\":{depth},\"calls\":{calls},\"elapsed_ns\":{},\"allocations\":{},\"allocated_bytes\":{}}}",
            elapsed.as_nanos(),
            ALLOCS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed)
        );
    }
}
