#!/usr/bin/env python3
"""Benchmark requested live heap payload for owned AAC initialization.

This explicit benchmark needs Git and rustc, but no FFmpeg or network. It
compares a saved own-code baseline with current code and checks bitwise output.
The meter excludes allocator overhead/RSS and does not measure hidden temporary
storage inside the system allocator during realloc.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
import json

ROOT = Path(__file__).resolve().parents[1]

METER = r"""
#![allow(dead_code)]
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicUsize,Ordering},f64::consts::PI};
static CURRENT:AtomicUsize=AtomicUsize::new(0);
static PEAK:AtomicUsize=AtomicUsize::new(0);
static CALLS:AtomicUsize=AtomicUsize::new(0);
struct Meter;
fn add(n:usize){let live=CURRENT.fetch_add(n,Ordering::SeqCst)+n;PEAK.fetch_max(live,Ordering::SeqCst);CALLS.fetch_add(1,Ordering::SeqCst);}
unsafe impl GlobalAlloc for Meter {
 unsafe fn alloc(&self,l:Layout)->*mut u8 {let p=unsafe{System.alloc(l)};if !p.is_null(){add(l.size());}p}
 unsafe fn alloc_zeroed(&self,l:Layout)->*mut u8 {let p=unsafe{System.alloc_zeroed(l)};if !p.is_null(){add(l.size());}p}
 unsafe fn dealloc(&self,p:*mut u8,l:Layout){unsafe{System.dealloc(p,l)};CURRENT.fetch_sub(l.size(),Ordering::SeqCst);}
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 {let next=unsafe{System.realloc(p,l,n)};if !next.is_null(){CURRENT.fetch_sub(l.size(),Ordering::SeqCst);add(n);}next}
}
#[global_allocator]static METER:Meter=Meter;
type Result<T>=std::result::Result<T,String>;
fn invalid(s:&str)->String{s.into()}
"""
MAIN = r"""
fn measure<T>(f:impl FnOnce()->T)->(T,usize,usize){let base=CURRENT.load(Ordering::SeqCst);PEAK.store(base,Ordering::SeqCst);CALLS.store(0,Ordering::SeqCst);let result=f();(result,PEAK.load(Ordering::SeqCst)-base,CALLS.load(Ordering::SeqCst))}
fn main(){
 for n in [120,128,960,1024] {
  let (a,ap,ac)=measure(||old_kbd(n,if n<200{6.0}else{4.0}));
  let (b,bp,bc)=measure(||new_kbd(n,if n<200{6.0}else{4.0}));
  assert!(a.iter().zip(&b).all(|(a,b)|a.to_bits()==b.to_bits()));assert_eq!(a.len(),b.len());assert!(bp<ap);assert!(bc<ac);
  println!("KBD {n}: requested-live peak {ap} -> {bp} bytes, allocation/reallocation calls {ac} -> {bc}, bitwise identical");
 }
 for n in [120,128,960,1024] {
  let (a,ap,ac)=measure(||old::Imdct::new(n).unwrap());let (b,bp,bc)=measure(||new::Imdct::new(n).unwrap());
  assert!(bp<ap);assert_eq!(ac,bc+1);
  let spectrum:Vec<f32>=(0..n).map(|i| ((i*7919%65536) as f32-32768.0)/32768.0).collect();let mut x=vec![0.0;2*n];let mut y=vec![0.0;2*n];
  a.inverse(&spectrum,&mut x).unwrap();b.inverse(&spectrum,&mut y).unwrap();assert!(x.iter().zip(&y).all(|(a,b)|a.to_bits()==b.to_bits()));
  println!("IMDCT {n}: requested-live peak {ap} -> {bp} bytes, allocation/reallocation calls {ac} -> {bc}, bitwise identical");
 }
}
"""

def kbd(source, name):
    start = source.index("fn kbd_window(")
    stop = source.index("#[derive(Clone)]", start)
    return source[start:stop].replace("fn kbd_window(", f"fn {name}(")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-ref", default="4ccf4852",
                        help="own-code Git revision before constructor allocation changes")
    args = parser.parse_args()
    def baseline(name):
        return subprocess.run(["git", "show", f"{args.baseline_ref}:crates/fvid-media/src/owned_aac/{name}.rs"],
                              cwd=ROOT, check=True, capture_output=True, text=True).stdout
    with tempfile.TemporaryDirectory(prefix="fvid-aac-allocation-bench-") as temp:
        temp = Path(temp)
        old = temp / "baseline_imdct.rs"
        old.write_text(baseline("aac_imdct"))
        modules = ""
        for name, path in [("memory", ROOT / "crates/fvid-media/src/owned_aac/memory.rs"),
                           ("old", old), ("new", ROOT / "crates/fvid-media/src/owned_aac/aac_imdct.rs")]:
            modules += f"#[path={json.dumps(str(path), ensure_ascii=False)}] mod {name};\n"
        source = temp / "measure.rs"
        source.write_text(METER + modules + kbd(baseline("aac_synthesis"), "old_kbd")
                          + kbd((ROOT / "crates/fvid-media/src/owned_aac/aac_synthesis.rs").read_text(), "new_kbd")
                          + MAIN)
        binary = temp / "measure"
        subprocess.run(["rustc", "--edition=2024", str(source), "-o", str(binary)], cwd=ROOT, check=True)
        subprocess.run([str(binary)], check=True)

if __name__ == "__main__":
    main()
