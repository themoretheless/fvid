//! Standalone measurement harness, compiled with rustc against the built libraries.
//! GlobalAlloc instrumentation is confined to this harness; not production code.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    io::Cursor,
};
thread_local! {
    static ACTIVE:Cell<bool>=const{Cell::new(false)};
    static LIVE:Cell<usize>=const{Cell::new(0)};
    static PEAK:Cell<usize>=const{Cell::new(0)};
    static COUNT:Cell<usize>=const{Cell::new(0)};
}
fn add(n: usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        COUNT.with(|c| c.set(c.get() + 1));
        LIVE.with(|l| {
            let n = l.get() + n;
            l.set(n);
            PEAK.with(|p| p.set(p.get().max(n)));
        });
    }
}
fn sub(n: usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        LIVE.with(|l| {
            l.set(
                l.get()
                    .checked_sub(n)
                    .expect("untracked deallocation inside allocation scope"),
            )
        });
    }
}
struct Measured;
// SAFETY: every allocation/deallocation is forwarded unchanged to System;
// counters use allocation-free thread-local cells and do not change ownership.
unsafe impl GlobalAlloc for Measured {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            add(l.size())
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            add(l.size())
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        sub(l.size());
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            sub(l.size());
            add(n)
        }
        q
    }
}
#[global_allocator]
static ALLOC: Measured = Measured;
fn begin() {
    LIVE.with(|c| c.set(0));
    PEAK.with(|c| c.set(0));
    COUNT.with(|c| c.set(0));
    ACTIVE.with(|c| c.set(true));
}
fn end() -> (usize, usize) {
    ACTIVE.with(|c| c.set(false));
    (COUNT.with(Cell::get), PEAK.with(Cell::get))
}
fn main() {
    let data =
        include_bytes!("../tests/fixtures/playback-errors/adts-packet-working-set/padded.aac");
    let check = std::env::args().any(|a| a == "--check");
    let mut bad = false;
    for backend in ["root", "owned"] {
        for iteration in 0..3 {
            let (packet, measurement) = if backend == "root" {
                let mut r = fvid::container::adts::StreamReader::open(Cursor::new(data)).unwrap();
                begin();
                let p = r.next_packet();
                let m = end();
                (p.unwrap().unwrap(), m)
            } else {
                let mut r =
                    fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(data)).unwrap();
                begin();
                let p = r.next_packet();
                let m = end();
                (p.unwrap().unwrap(), m)
            };
            assert_eq!(packet.as_slice(), &data[7..1049]);
            let (allocations, peak_bytes) = measurement;
            println!(
                "{{\"backend\":\"{backend}\",\"iteration\":{iteration},\"allocations\":{allocations},\"peak_bytes\":{peak_bytes},\"payload_bytes\":{}}}",
                packet.len()
            );
            bad |= allocations > 2 || peak_bytes > packet.len() + 32;
        }
    }
    if check {
        assert!(
            !bad,
            "single-block delivery retains a duplicate frame allocation"
        );
    }
}
