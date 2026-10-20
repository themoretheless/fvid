//! Explicit allocation diagnostic, separate from timing and ordinary tests.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
struct Count;
static ALLOC: AtomicUsize = AtomicUsize::new(0);
static REALLOC: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn add(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            ALLOC.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(layout.size(), Relaxed);
            add(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            ALLOC.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(layout.size(), Relaxed);
            add(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, old, size) };
        if !p.is_null() {
            REALLOC.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(size, Relaxed);
            if size >= old.size() {
                add(size - old.size());
            } else {
                LIVE.fetch_sub(old.size() - size, Relaxed);
            }
        }
        p
    }
}
#[global_allocator]
static GLOBAL: Count = Count;
fn main() {
    let path = std::env::args().nth(1).expect("FLAC input");
    let data = std::fs::read(&path).unwrap();
    let initial = LIVE.load(Relaxed);
    PEAK.store(initial, Relaxed);
    ALLOC.store(0, Relaxed);
    REALLOC.store(0, Relaxed);
    REQUESTED.store(0, Relaxed);
    let flac = fvid_media::owned_flac::Flac::parse(&data, &Default::default()).unwrap();
    let stats = (
        ALLOC.load(Relaxed),
        REALLOC.load(Relaxed),
        REQUESTED.load(Relaxed),
        PEAK.load(Relaxed) - initial,
        LIVE.load(Relaxed) - initial,
    );
    println!(
        "{}",
        serde_json::json!({"file":path,"frames":flac.frames.len(),"samples":flac.samples(),"index_capacity":flac.frames.capacity(),"allocation_calls":stats.0,"reallocation_calls":stats.1,"requested_bytes_total":stats.2,"peak_live_requested_bytes":stats.3,"retained_requested_bytes":stats.4,"scope":"Allocation requests only; not allocator metadata/RSS, and not a timing benchmark."})
    );
}
