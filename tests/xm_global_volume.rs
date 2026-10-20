#![cfg(feature="player")]
use fvid::audio::{AudioStream,AudioDecode};
use std::{io::Cursor,path::{Path,PathBuf},time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn base()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-global-volume")}
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(bytes:Vec<u8>,channels:u16)->(Vec<(Duration,Vec<u8>)>,u32){let mut source=open(bytes);let mut decoder=fvid::codec::xm_decoder::XmDecoder::new(source.extra_data(),source.sample_rate(),channels).unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}(out,decoder.ignored().effects)}
#[test]
fn native_xm_global_volume_changes_the_previously_ignored_control(){let base=base();let(actual,ignored)=render(std::fs::read(base.join("level-0.xm")).unwrap(),2);let(without,_)=render(std::fs::read(base.join("level-0-reference.xm")).unwrap(),2);assert!(actual!=without&&ignored==0,"native Gxx must now change gain and be reported supported");}
#[test]
fn native_xm_global_volume_acceptance_scales_all_channels_before_clipping(){
    let base=base();let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let source=open(bytes.clone());let reference=open(control.clone());assert_eq!(source.duration(),reference.duration(),"{name}: unrelated flow mismatch");let mut stereo_wanted=Vec::new();
        for channels in [1,2]{let(baseline,reference_ignored)=render(control.clone(),channels);let(actual,ignored)=render(bytes.clone(),channels);assert_eq!(reference_ignored,0);assert_eq!(actual.len(),baseline.len());if ignored!=0{failures.push(format!("{name}/{channels}: Gxxstill ignored"));}let mut wanted=baseline.clone();
            if case["oracle"]=="column-steady"{wanted=actual.clone();let guard=64*4*usize::from(channels);for(i,(_,data))in wanted.iter_mut().enumerate(){data[guard..].copy_from_slice(&baseline[i].1[guard..]);}}
            else{for(i,(_,data))in wanted.iter_mut().enumerate(){let gain=case["gains"][i].as_u64().unwrap() as f32/64.;for sample in data.chunks_exact_mut(4){let value=f32::from_le_bytes(sample.try_into().unwrap())*gain;sample.copy_from_slice(&value.to_le_bytes());}}}
            if actual!=wanted{failures.push(format!("{name}/{channels}: Gxxgain/scope/headroom differs from independent reference"));}if channels==2{stereo_wanted=wanted;}
        }
        let row=case["seek"].as_u64().unwrap() as usize;
        for(target,suffix)in[(Duration::from_millis(row as u64*120),&stereo_wanted[row..]),(reference.duration().unwrap(),&stereo_wanted[stereo_wanted.len()..])]{if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes.clone())),&stereo_wanted,target,suffix))).is_err(){failures.push(format!("{name}: actual Gxxplayback/seek differs"));}}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
