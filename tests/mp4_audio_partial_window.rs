#![cfg(feature = "player")]
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_transitions.rs"] mod transitions;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-audio-partial-window").join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"].as_array().unwrap().clone()
}
#[test]
fn full_native_block_controls_keep_exact_pcm() {
    for c in cases().into_iter().filter(|c| c["file"].as_str().unwrap().contains("control")) {
        let mut r = Mp4AudioReader::open(Cursor::new(bytes(c["file"].as_str().unwrap())), Default::default()).unwrap();
        let got: Vec<u8> = transitions::collect(&mut r).into_iter().flat_map(|f|f.3).collect();
        assert_eq!(got, bytes(c["oracle"].as_str().unwrap()));
    }
}
#[test]
fn positive_partial_terminal_windows_clip_pcm_and_seek_suffix() {
    let mut failures=Vec::new();
    for c in cases().into_iter().filter(|c| c["file"].as_str().unwrap().contains("partial")) {
        let name=c["file"].as_str().unwrap();
        let mut r=Mp4AudioReader::open(Cursor::new(bytes(name)),Default::default()).unwrap();
        let want=bytes(c["oracle"].as_str().unwrap());
        assert_eq!(r.duration(),Some(Duration::from_nanos(c["frames"].as_u64().unwrap()*125000)));
        for phase in 0..3 {
            let skip=if phase==2 {1} else {0};
            if phase==1 {r.rewind();}
            if phase==2 {r.seek_time(Duration::from_nanos(125000));}
            let got:Vec<u8>=transitions::collect(&mut r).into_iter().flat_map(|f|f.3).collect();
            if got!=want[skip*4..] {failures.push(format!("{name} phase{phase}: {} bytes expected {}",got.len(),want.len()-skip*4));}
        }
        r.seek_time(r.duration().unwrap());
        let eof:Vec<u8>=transitions::collect(&mut r).into_iter().flat_map(|f|f.3).collect();
        if !eof.is_empty() {failures.push(format!("{name} EOF: {} unexpected bytes",eof.len()));}
    }
    assert!(failures.is_empty(),"PCM beyond positive terminal window: {failures:?}");
}
#[test]
fn authored_timing_retains_coded_packets_and_decodable_video() {
    for c in cases() {
        let name=c["file"].as_str().unwrap();
        let data=bytes(name);
        let mut r=fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(data.clone()),Default::default()).unwrap();
        let ti=r.tracks().iter().position(|t|t.handler==*b"soun").unwrap();
        let control=if name.starts_with("ima") {"ima-control.mp4"}else{"alac-control.mp4"};
        let mut base=fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(bytes(control)),Default::default()).unwrap();
        let bt=base.tracks().iter().position(|t|t.handler==*b"soun").unwrap();
        let mut a=Vec::new();let mut b=Vec::new();
        for i in 0..3 {
            assert_eq!(r.tracks()[ti].samples.get(i).unwrap().duration,c["durations"][i].as_u64().unwrap() as u32);
            r.read_packet(ti,i,&mut a).unwrap();base.read_packet(bt,i,&mut b).unwrap();assert_eq!(a,b);
        }
        let mut v=fvid::playback_native::NativeReader::software(Cursor::new(data),32<<20).unwrap();
        let mut count=0;while v.read_frame_raw().unwrap().is_some(){count+=1;}assert_eq!(count,3);
    }
}

#[test]
fn real_audio_worker_respects_partial_tail_and_eof() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc,Mutex};
    use transitions::{Capture,Trace,ended};
    for c in cases() {
        let name=c["file"].as_str().unwrap();
        let r=Mp4AudioReader::open(Cursor::new(bytes(name)),Default::default()).unwrap();
        let end=r.duration().unwrap();
        let trace=Arc::new(Mutex::new(Trace::default()));let shared=trace.clone();
        let mut worker=AudioPlayback::start(Box::new(r),move||Box::new(Capture::new(shared)));
        worker.play();ended(&worker,0);
        let got:Vec<u8>=trace.lock().unwrap().packets.iter().flat_map(|p|p.3.iter().copied()).collect();
        assert_eq!(got,bytes(c["oracle"].as_str().unwrap()),"{name}: worker");
        trace.lock().unwrap().packets.clear();
        worker.seek(end);ended(&worker,worker.generation());
        assert!(trace.lock().unwrap().packets.is_empty(),"{name}: worker EOF");
    }
}
