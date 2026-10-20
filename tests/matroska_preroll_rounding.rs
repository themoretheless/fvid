#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
#[test]
fn sample_grid_preroll_quantization_does_not_replace_the_first_audible_neighbor() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-preroll-rounding");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong=Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let expected=std::fs::read(base.join(format!("{name}-expected.f32le"))).unwrap();
        let raw=std::fs::read(base.join(format!("{name}-source.f32le"))).unwrap();
        if case["origin_ns"].as_i64().unwrap()==0 {
            let mut owned=Vec::new();let result=fvid_media::owned_matroska_pcm::decode_matroska_pcm(Cursor::new(bytes.clone()),&mut owned,None,&Default::default());
            if expected.is_empty(){assert!(result.err().unwrap().to_string().contains("contains no samples"));}
            else{result.unwrap();assert_eq!(owned,expected,"{name} independent owned sample-count reference");}
        }
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()),32<<20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap();
        let encoded=source.next_packet().unwrap().unwrap();
        assert_eq!(encoded.pts,case["origin_ns"].as_i64().unwrap()-case["delay_ns"].as_i64().unwrap(),"{name}: preserve literal CodecDelay timestamp subtraction");
        let mut decoder=source.make_decoder().unwrap();let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();assert_eq!(frame.packet.data,raw);
        let result=source.present_decoded(frame.packet,frame.source_pts);
        let actual=match result {Ok(Some(packet))=>packet,Ok(None) if expected.is_empty()=>{
            let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();capture::worker_check_with_seek(Box::new(source),&[],Some(Duration::ZERO));continue;
        },other=>{wrong.push(format!("{name}: expected audible packet, got {}",if other.is_err(){"error"}else{"none"}));continue;}};
        if actual.data!=expected {wrong.push(format!("{name}: {} instead of{} frames",actual.data.len()/4,expected.len()/4));continue;}
        let at=Duration::from_nanos(case["pts_ns"].as_u64().unwrap());assert_eq!(actual.presentation_time(),at,"{name}");
        let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();capture::worker_check_with_seek(Box::new(source),&[(at,expected)],Some(Duration::ZERO));
    }
    assert!(wrong.is_empty(),"quantized preroll discarded a nonpadding neighbor: {wrong:?}");
}

#[test]
fn explicit_positive_opus_and_vorbis_seek_floors_still_ceil() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-padding-rounding");
    for (name,channels,frames) in [("opus-control",1,959),("vorbis-control",2,79)] {
        let bytes=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
        assert_eq!(source.seek_to(1),1);let mut decoder=source.make_decoder().unwrap();
        loop {
            let encoded=source.next_packet().unwrap().unwrap();
            if let Some(frame)=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap(){
                if let Some(packet)=source.present_decoded(frame.packet,frame.source_pts).unwrap(){
                    assert_eq!(packet.data.len()/4/channels,frames,"{name} strict seek1ns");
                    assert_eq!(packet.presentation_time(),Duration::from_nanos(20833));break;
                }
            }
        }
    }
}

#[test]
fn codec_delay_clock_overflow_remains_a_precise_zero_output_refusal() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-preroll-rounding");
    let bytes=std::fs::read(base.join("overflow.mkv")).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()),32<<20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
    capture::worker_refusal(Box::new(source),"codec delay timestamp overflow");
}

#[test]
fn opus_and_vorbis_zero_origin_quantization_preserve_the_same_known_two_frame_preroll() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-preroll-rounding");
    let control=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-padding-rounding/opus-control.mkv");
    let mut control=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(std::fs::read(control).unwrap()),Default::default()).unwrap();
    let mut decoder=control.make_decoder().unwrap();let encoded=control.next_packet().unwrap().unwrap();
    let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
    let opus_control=control.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap().data;
    for (name,channels,full,expected) in [("opus-delay-two",1,960,958),("vorbis-delay-two",2,448,446)] {
        let bytes=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()),32<<20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap();
        let mut decoder=source.make_decoder().unwrap();let mut decoded=0;let mut rendered=0;let mut intervals=Vec::new();
        while let Some(encoded)=source.next_packet().unwrap(){
            if let Some(frame)=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap(){
                decoded+=frame.packet.data.len()/4/channels;
                if let Some(packet)=source.present_decoded(frame.packet,frame.source_pts).unwrap(){
                    assert!(packet.data.chunks_exact(4).all(|b|f32::from_le_bytes(b.try_into().unwrap()).is_finite()),"{name}");
                    if name.starts_with("opus") {assert_eq!(packet.data,opus_control[8..]);}
                    else {assert!(packet.data.chunks_exact(4).all(|b|f32::from_le_bytes(b.try_into().unwrap())==0.0));}
                    if intervals.is_empty(){assert_eq!(packet.presentation_time(),Duration::ZERO);}
                    rendered+=packet.data.len()/4/channels;intervals.push((packet.presentation_time(),packet.data));
                }
            }
        }
        assert_eq!(decoded,full,"{name} complete native control");assert_eq!(rendered,expected,"{name} two-frame trim");
        let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();capture::worker_check_with_seek(Box::new(source),&intervals,Some(Duration::ZERO));
    }
}
