#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader {
    fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()
}
#[test]
fn xm_rows_share_one_absolute_sample_grid_without_cumulative_rounding_loss(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-sample-grid");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){
        let name=case["file"].as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();
        let mut source=open(bytes.clone());let rate=u64::from(source.sample_rate());let stride=usize::from(source.channels())*4;
        let mut engine=source.make_decoder().unwrap();let mut encoded=Vec::new();let mut expected=Vec::new();let mut all=Vec::new();let mut cursor=0;
        while let Some(p)=source.next_packet().unwrap(){
            let f=engine.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();
            let frames=f.packet.data.len()/stride;
            let end=(p.pts as u64+p.duration as u64)*rate/1000;
            if end-cursor!=frames as u64 { failures.push(format!("{name}: row{} has{frames} frames instead of{}",encoded.len(),end-cursor)); }
            let at=Duration::from_nanos(cursor*1_000_000_000/rate);
            if f.packet.presentation_time()!=at {failures.push(format!("{name}: row{} is off the absolute sample grid",encoded.len()));}
            cursor=end;all.extend_from_slice(&f.packet.data);expected.push((f.packet.presentation_time(),f.packet.data));encoded.push(p);
        }
        assert!(all.chunks_exact(4).any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01),"{name}: silent control");
        // Score rows after the first are empty; render the held note in one call
        // and then apply the playback tail release separately.
        // in one call must preserve the same oscillator/sample history and PCM.
        let release=encoded.last().unwrap();let end_ms=(release.pts+release.duration) as u64;let hold_frames=end_ms*rate/1000;
        let mut grouped=source.make_decoder().unwrap();let first=&encoded[0];
        let mut held=grouped.decode_packet(&first.data,0,release.pts as u64).unwrap().unwrap();
        let tail=grouped.decode_packet(&release.data,release.pts,release.duration as u64).unwrap().unwrap();
        held.packet.data.extend_from_slice(&tail.packet.data);
        if all.len()!=hold_frames as usize*stride || all!=held.packet.data {
            failures.push(format!("{name}: row splitting changes continuous held-note PCM"));
        }
        capture::worker_check_with_seek(Box::new(open(bytes.clone())),&expected,source.duration());
        let mid=encoded.len()/2;
        capture::worker_check_with_seek_expected(Box::new(open(bytes)),&expected,
            Duration::from_millis(encoded[mid].pts as u64+1),&expected[mid..]);
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
