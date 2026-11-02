#![cfg(feature="player")]
#[path="support/audio_transitions.rs"]mod transitions;
#[path="support/rational_audio_seek.rs"]mod rational;
use fvid::{audio::AudioStream,playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,time::Duration};
fn bytes(n:&str)->Vec<u8>{std::fs::read(format!("{}/tests/fixtures/playback-errors/mp4-mixed-coded-seek/{n}",env!("CARGO_MANIFEST_DIR"))).unwrap()}
fn open()->Mp4AudioReader<Cursor<Vec<u8>>>{Mp4AudioReader::open(Cursor::new(bytes("pcm-phase.mp4")),Default::default()).unwrap()}
#[test]
fn authored_pcm_and_short_video_validate_the_fractional_stamp_fixture(){
 let mut r=open();assert_eq!((r.sample_rate(),r.channels(),r.track().timescale),(44100,2,14112000));assert_eq!(r.duration(),Some(rational::duration(80,44100)));
 let full=rational::collect(&mut r);let pcm:Vec<_>=full.iter().flat_map(|p|p.data.iter().copied()).collect();assert_eq!(pcm,bytes("pcm-phase.f32le"));assert_eq!(full.iter().map(rational::Packet::frames).sum::<usize>(),80);assert_eq!(full[0].point(0),(0,1));
 let mut v=fvid::playback_native::NativeReader::software(Cursor::new(bytes("pcm-phase.mp4")),32<<20).unwrap();for _ in 0..6{assert_eq!(v.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(),vec![130;16*16*3]);}assert!(v.read_frame_raw().unwrap().is_none());
}
#[test]
fn raw_media_tick_clipping_retains_the_exact_native_fraction(){
 let mut r=open();let full=rational::collect(&mut r);let golden=bytes("pcm-phase.f32le");
 // 12800 is exactly native sample40 on the14112000Hz media clock.
 // No Duration conversion or rounded wall-time request participates here.
 for tick in [1i64,7056,12800,25280,25600]{
  let first=(tick as u128*44100).div_ceil(14112000).min(80)as usize;
  let at=r.seek_to(tick);assert_eq!(at,0,"raw contract still lands at the source packet");let got=rational::collect(&mut r);
  let pcm:Vec<_>=got.iter().flat_map(|p|p.data.iter().copied()).collect();assert_eq!(pcm,&golden[first*8..],"raw tick{tick}:PCM suffix");
  if first==80{assert!(got.is_empty());continue;}
  assert_eq!(got[0].num*44100,first as u128*got[0].den,"raw tick{tick}:native sample label must be exact,not floored nanoseconds");
  let wanted=rational::suffix(&full,Duration::from_nanos((tick as u128*1_000_000_000/14112000)as u64),(80,44100));
  // The direct raw-index equality above is authoritative even when flooring a
  // display Duration would point slightly before an exact native boundary.
  assert!(!wanted.is_empty());
 }
}
