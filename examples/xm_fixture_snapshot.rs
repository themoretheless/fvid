//! Explicit local audit helper: writes native XM PCM to stdout and metadata to stderr.
#[cfg(feature="player")]
fn main() {
    use fvid::audio::AudioStream;
    use std::io::Write;
    let path=std::env::args().nth(1).expect("synthetic XM path");
    let mut source=fvid::playback_xm::XmAudioReader::open(std::fs::File::open(path).unwrap(),Default::default()).unwrap();
    let mut decoder=source.make_decoder().unwrap();let mut out=std::io::stdout().lock();let mut starts=Vec::new();let mut frames=0;
    while let Some(p)=source.next_packet().unwrap(){
        if let Some(f)=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap(){
            if let Some(packet)=source.present_decoded(f.packet,f.source_pts).unwrap(){
                starts.push(packet.presentation_time().as_nanos());frames+=packet.data.len()/4/usize::from(source.channels());out.write_all(&packet.data).unwrap();
            }
        }
    }
    source.validate_eof().unwrap();
    eprintln!("{}",serde_json::json!({"sample_rate":source.sample_rate(),"channels":source.channels(),"frames":frames,"packet_start_ns":starts}));
}
#[cfg(not(feature="player"))]
fn main(){panic!("requires --features player");}
