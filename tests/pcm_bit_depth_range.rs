#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path};
#[test]
fn declared_pcm_widths_above_u8_do_not_alias_supported_sample_formats() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/pcm-bit-depth-range");
    let expected=std::fs::read(base.join("expected.f32le")).unwrap();
    let mut admitted=Vec::new();
    for (name,bits,valid) in [("int16",16,true),("float32",32,true),("int272",272,false),("float288",288,false)] {
        let media=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(media.clone()),32<<20).unwrap();
        for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(media),Default::default()).unwrap();
        assert_eq!(source.bits_per_sample(),bits);
        match source.make_decoder() {
            Ok(mut decoder) if valid=>{
                let encoded=source.next_packet().unwrap().unwrap();
                let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
                assert_eq!(frame.packet.data,expected);
            }
            Ok(_)=>admitted.push(name),
            Err(error) if !valid=>assert!(error.to_string().contains(&bits.to_string()),"{error}"),
            Err(error)=>panic!("valid PCM control refused: {error}"),
        }
    }
    assert!(admitted.is_empty(),"unsupported declared widths silently admitted: {admitted:?}");
}
