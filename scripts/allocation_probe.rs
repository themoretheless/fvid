//! Standalone measurement harness, intentionally outside the unsafe-forbidden library.
//! Counts Rust allocator calls on a single-threaded synchronous workload, not native/GPU allocations.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use rmcp::ServerHandler;
struct Counting;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
fn record(n: usize) { if ACTIVE.load(Relaxed) { CALLS.fetch_add(1, Relaxed); BYTES.fetch_add(n, Relaxed); } }
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { record(l.size()); unsafe { System.alloc(l) } }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { record(l.size()); unsafe { System.alloc_zeroed(l) } }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 { record(n); unsafe { System.realloc(p,l,n) } }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p,l) } }
}
#[global_allocator] static ALLOCATOR: Counting = Counting;
fn measure(name: &str, f: impl FnOnce()) {
    CALLS.store(0,Relaxed); BYTES.store(0,Relaxed); ACTIVE.store(true,Relaxed);
    f(); ACTIVE.store(false,Relaxed);
    println!("{name}: calls={} requested_bytes={}",CALLS.load(Relaxed),BYTES.load(Relaxed));
}
fn main() {
    let server=fvid::mcp::Server::new(".").unwrap();
    std::hint::black_box(server.get_tool("fvid_process_y4m"));
    measure("1000 known MCP lookups",|| for _ in 0..1000 { std::hint::black_box(server.get_tool("fvid_process_y4m")); });
    measure("1000 unknown MCP lookups",|| for _ in 0..1000 { std::hint::black_box(server.get_tool("missing")); });
    for n in [1,1000] {
        let mut data=b"YUV4MPEG2 W64 H32 C420\n".to_vec();
        for _ in 0..n { data.extend_from_slice(b"FRAME\n"); data.extend_from_slice(&[7;3072]); }
        measure(&format!("CPU {n} frames"),|| { std::hint::black_box(fvid::process(std::io::Cursor::new(&data),std::io::sink(),fvid::Transform { horizontal:true,..Default::default() },4096).unwrap()); });
    }
}
