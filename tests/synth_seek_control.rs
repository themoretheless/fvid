#![cfg(feature="player")]
use fvid::audio::{AudioStream,AudioDecode,AudioPacket,AudioBackend,AudioSpec,AudioError,EncodedPacket};
use std::{io::Cursor,path::Path,time::Duration,sync::{Arc,Mutex,Condvar,mpsc,atomic::{AtomicUsize,Ordering}}};
struct Gate { state:Mutex<u8>,wake:Condvar,calls:AtomicUsize,resets:AtomicUsize,entered:mpsc::Sender<()> }
impl Gate {fn open(&self,state:u8){*self.state.lock().unwrap()=state;self.wake.notify_all();}}
struct Gated { inner:Box<dyn AudioDecode>,gate:Arc<Gate> }
impl AudioDecode for Gated {
    fn reset(&mut self){self.gate.resets.fetch_add(1,Ordering::SeqCst);self.inner.reset();}
    fn decode_encoded(&mut self,data:&[u8],pts:u64,duration:u64)->fvid::Result<Option<AudioPacket>>{
        let call=self.gate.calls.fetch_add(1,Ordering::SeqCst)+1;
        if call<=2 {
            let _=self.gate.entered.send(());
            let mut state=self.gate.state.lock().unwrap();
            while *state<call as u8 {state=self.gate.wake.wait(state).unwrap();}
        }
        self.inner.decode_encoded(data,pts,duration)
    }
}
struct Source { inner:Box<dyn AudioStream>,gate:Arc<Gate> }
impl AudioStream for Source {
    fn codec(&self)->&str{self.inner.codec()} fn timescale(&self)->u32{self.inner.timescale()}
    fn sample_rate(&self)->u32{self.inner.sample_rate()} fn channels(&self)->u16{self.inner.channels()}
    fn bits_per_sample(&self)->u16{self.inner.bits_per_sample()} fn duration(&self)->Option<Duration>{self.inner.duration()}
    fn audio_tracks(&self)->Vec<fvid::audio::AudioTrack>{self.inner.audio_tracks()}
    fn extra_data(&self)->&[u8]{self.inner.extra_data()} fn next_packet(&mut self)->fvid::Result<Option<EncodedPacket>>{self.inner.next_packet()}
    fn rewind(&mut self){self.inner.rewind()} fn seek_to(&mut self,pts:i64)->i64{self.inner.seek_to(pts)}
    fn present_decoded(&self,p:AudioPacket,pts:i64)->fvid::Result<Option<AudioPacket>>{self.inner.present_decoded(p,pts)}
    fn make_decoder(&self)->fvid::Result<Box<dyn AudioDecode>>{Ok(Box::new(Gated{inner:self.inner.make_decoder()?,gate:self.gate.clone()}))}
    fn reset_decoder(&mut self,decoder:&mut Box<dyn AudioDecode>)->fvid::Result<()>{self.inner.reset_decoder(decoder)}
}
struct Backend { paused:mpsc::Sender<()>,at:Duration,captured:Arc<Mutex<Vec<(Duration,Vec<u8>)>>> }
impl AudioBackend for Backend {
    fn start(&mut self,_:AudioSpec)->Result<(),AudioError>{Ok(())}
    fn push(&mut self,p:AudioPacket)->Result<(),AudioError>{self.at=p.presentation_time()+Duration::from_nanos((p.data.len()/8) as u64*1_000_000_000/44100);self.captured.lock().unwrap().push((p.presentation_time(),p.data));Ok(())}
    fn position(&self)->Duration{self.at} fn flush(&mut self,at:Duration)->Result<(),AudioError>{self.at=at;self.captured.lock().unwrap().clear();Ok(())}
    fn pause(&mut self)->Result<(),AudioError>{let _=self.paused.send(());Ok(())}
    fn resume(&mut self)->Result<(),AudioError>{Ok(())} fn stop(&mut self)->Result<(),AudioError>{Ok(())}
}
#[test]
fn commands_are_serviced_between_native_synthesis_seek_history_windows(){
    let root=Path::new(env!("CARGO_MANIFEST_DIR"));let base=root.join("tests/fixtures/playback-errors/synth-seek-control");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){
      for reseek in [false,true] {
        let kind=case["kind"].as_str().unwrap();let bytes=std::fs::read(root.join(case["source"].as_str().unwrap())).unwrap();
        let mut baseline:Box<dyn AudioStream>=match kind {"midi"=>Box::new(fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap()),"xm"=>Box::new(fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap()),_=>unreachable!()};
        let target=Duration::from_millis(case["target_ms"].as_u64().unwrap());
        let final_target=if reseek {Duration::from_millis(if kind=="midi"{500}else{960})}else{target};
        let mut expected=Vec::new();let mut decoder=baseline.make_decoder().unwrap();
        while let Some(p)=baseline.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();if baseline.time_of(p.pts)>=final_target {expected.push((f.packet.presentation_time(),f.packet.data));}}
        baseline.rewind();let inner=baseline;
        let captured=Arc::new(Mutex::new(Vec::new()));let sink=captured.clone();
        let (entered_tx,entered_rx)=mpsc::channel();let gate=Arc::new(Gate{state:Mutex::new(0),wake:Condvar::new(),calls:AtomicUsize::new(0),resets:AtomicUsize::new(0),entered:entered_tx});
        let (paused_tx,paused_rx)=mpsc::channel();let mut playback=fvid::audio_thread::AudioPlayback::start(Box::new(Source{inner,gate:gate.clone()}),move||Box::new(Backend{paused:paused_tx,at:Duration::ZERO,captured:sink}));
        playback.seek(target);playback.play();
        let entered=entered_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        if entered {if reseek {playback.seek(final_target);}else{playback.pause();}gate.open(1);}
        let serviced=if reseek {
            entered&&entered_rx.recv_timeout(Duration::from_millis(500)).is_ok()&&gate.resets.load(Ordering::SeqCst)==2
        }else{entered&&paused_rx.recv_timeout(Duration::from_millis(500)).is_ok()};
        let calls=gate.calls.load(Ordering::SeqCst);
        // Always unblock the second gate before joining, including a failing run.
        gate.open(2);playback.play();
        let deadline=std::time::Instant::now()+Duration::from_secs(2);let mut ended=false;
        while std::time::Instant::now()<deadline {
            match playback.poll(){Some(fvid::audio_thread::AudioEvent::Ended(g)) if g==if reseek{2}else{1}=>{ended=true;break;},Some(fvid::audio_thread::AudioEvent::Error(e))=>{failures.push(format!("{kind}: resume failed:{e}"));break;},_=>{}}
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(playback);
        let actual=captured.lock().unwrap();
        if !ended || *actual!=expected {failures.push(format!("{kind}: resumed preroll does not preserve exact continuous PCM suffix"));}
        if !entered || !serviced || calls!=if reseek{2}else{1} {failures.push(format!("{kind}: command not serviced between history windows (reseek={reseek},entered={entered},serviced={serviced},decoder_calls={calls})"));}
      }
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
