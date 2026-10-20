#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path};
#[path="support/audio_capture.rs"] mod capture;
fn open(kind:&str,bytes:Vec<u8>)->Box<dyn AudioStream>{
    match kind {
        "midi"=>Box::new(fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()),
        "xm"=>Box::new(fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()),
        _=>unreachable!()
    }
}
fn decode(source:&mut dyn AudioStream)->Vec<(std::time::Duration,Vec<u8>)>{
    let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();
    while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out
}
#[test]
fn synthesized_held_notes_release_in_the_reserved_eof_tail(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/synth-tail-release");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){
        let kind=case["kind"].as_str().unwrap();let held=std::fs::read(base.join(case["held"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();
        let mut source=open(kind,held.clone());let actual=decode(&mut *source);let mut reference=open(kind,control.clone());let expected=decode(&mut *reference);
        assert!(expected[0].1.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01),"{kind}: silent control");
        let suffix=&expected[actual.len()-1].1;assert!(suffix.rchunks_exact(4).take(200).all(|s|f32::from_le_bytes(s.try_into().unwrap())==0.),"{kind}: explicit release did not reach silence");
        if actual!=expected[..actual.len()]{failures.push(format!("{kind}: held note never releases in reserved EOF tail"));}
        capture::worker_check_with_seek(open(kind,control),&expected,reference.duration());
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek(open(kind,held.clone()),&expected[..actual.len()],source.duration()))).is_err(){failures.push(format!("{kind}: actual worker cuts a held note at EOF"));}
        // Seeking into the release phase must rebuild the same released history.
        let index=if kind=="midi"{2}else{actual.len()-1};
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(open(kind,held),&expected[..actual.len()],expected[index].0,&expected[index..actual.len()]))).is_err(){failures.push(format!("{kind}: release seek differs from explicit note-off"));}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
