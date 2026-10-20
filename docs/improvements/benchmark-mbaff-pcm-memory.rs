//! Actual native decoder allocation probe; compile with an existing fvid-codecs rlib.
//! Decoder latency from a debug library is diagnostic only; acceptance uses allocator metrics.
use fvid_codecs::codec::avc_decoder::AvcDecoder;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};
struct Tracked;
static TRACK: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Tracked {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            if TRACK.load(Ordering::Relaxed) {
                PEAK.fetch_max(live, Ordering::Relaxed);
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            }
        }
        pointer
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, layout) }
    }
}
#[global_allocator]
static GLOBAL: Tracked = Tracked;
fn main() {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("fixture directory"));
    let maximum = std::env::args()
        .nth(2)
        .map(|v| v.parse::<usize>().unwrap())
        .unwrap_or(128);
    for side in [64, 128, 256].into_iter().filter(|s| *s <= maximum) {
        for field in [false, true] {
            let name = format!("{side}-{}", if field { "field" } else { "frame" });
            let config = fs::read(root.join(format!("{name}.avcc"))).unwrap();
            let packet = fs::read(root.join(format!("{name}-0.packet"))).unwrap();
            let expected: Vec<Vec<u16>> = ["y", "cb", "cr"]
                .into_iter()
                .map(|p| {
                    fs::read(root.join(format!("{name}-0.{p}")))
                        .unwrap()
                        .chunks_exact(2)
                        .map(|v| u16::from_le_bytes([v[0], v[1]]))
                        .collect()
                })
                .collect();
            let mut decoder = AvcDecoder::new(&config, 64 << 20).unwrap();
            let warm = decoder.decode(&packet).unwrap().unwrap();
            drop(warm);
            decoder.reset();
            CALLS.store(0, Ordering::Relaxed);
            BYTES.store(0, Ordering::Relaxed);
            let baseline = LIVE.load(Ordering::Relaxed);
            PEAK.store(baseline, Ordering::Relaxed);
            TRACK.store(true, Ordering::Relaxed);
            let start = Instant::now();
            let picture = decoder
                .decode(std::hint::black_box(&packet))
                .unwrap()
                .unwrap();
            let elapsed = start.elapsed();
            TRACK.store(false, Ordering::Relaxed);
            assert_eq!(picture.component_depths(), [8, 10, 10]);
            assert_eq!(picture.y, expected[0]);
            assert_eq!(picture.cb, expected[1]);
            assert_eq!(picture.cr, expected[2]);
            println!(
                "{{\"side\":{side},\"field\":{field},\"allocations\":{},\"allocated_bytes\":{},\"peak_requested_live_delta\":{},\"debug_elapsed_ns_diagnostic\":{}}}",
                CALLS.load(Ordering::Relaxed),
                BYTES.load(Ordering::Relaxed),
                PEAK.load(Ordering::Relaxed).saturating_sub(baseline),
                elapsed.as_nanos()
            );
        }
    }
}
