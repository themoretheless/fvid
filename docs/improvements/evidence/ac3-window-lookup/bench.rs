use std::{hint::black_box,time::Instant};
pub struct Frame {
    pub start: usize,
    pub size: usize,
    pub pts: u64,
    /// The frame's own geometry, kept so a test can name what was read rather
    /// than a copy of the numbers the file states.
    pub rate: u32,
    pub channels: u16,
    pub samples: u32,
}
const SAMPLES_PER_FRAME:u32=1536;
mod before {use super::{Frame,SAMPLES_PER_FRAME};include!("before.rs");pub fn lookup(f:&[Frame],p:i64)->usize{raw_frame_window(f,p)}}
mod candidate {use super::{Frame,SAMPLES_PER_FRAME};include!("../../../../src/playback/audio/ac3_window.rs");pub fn lookup(f:&[Frame],p:i64)->usize{raw_frame_window(f,p)}}
fn measure(f:&[Frame],queries:&[i64],lookup:fn(&[Frame],i64)->usize)->(u128,u64){let started=Instant::now();let mut sum=0u64;for &p in queries{sum=sum.wrapping_add(black_box(lookup(black_box(f),black_box(p)))as u64);}black_box(sum);(started.elapsed().as_nanos(),sum)}
fn main(){println!("{{\"frame_bytes\":{},\"scope\":\"actual window helper only;not playback throughput\"}}",std::mem::size_of::<Frame>());for count in [1usize,4,16,128,1024,8192,65536]{let mut pts=0;let frames:Vec<_>=(0..count).map(|i|{let samples=[256,512,768,1536][i%4];let frame=Frame{start:i*384,size:384,pts,rate:48000,channels:1,samples};pts+=u64::from(samples);frame}).collect();let n=8192;let queries:Vec<_>=(0..n).map(|i|{let index=if count<=n{i%count}else{i*count/n+i%((count/n).min(4))};frames[index].pts as i64}).collect();for round in 0..5{let(old,new)=if round%2==0{(measure(&frames,&queries,before::lookup),measure(&frames,&queries,candidate::lookup))}else{let new=measure(&frames,&queries,candidate::lookup);let old=measure(&frames,&queries,before::lookup);(old,new)};assert_eq!(old.1,new.1);println!("{{\"frames\":{},\"queries\":{},\"round\":{},\"baseline_ns\":{},\"candidate_ns\":{},\"checksum\":{}}}",count,n,round,old.0,new.0,old.1);}}}
