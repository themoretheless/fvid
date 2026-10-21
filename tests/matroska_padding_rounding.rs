#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn packet(bytes:Vec<u8>)->(fvid::audio::AudioPacket,Vec<u8>) {
    let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
    let mut decoder=source.make_decoder().unwrap();let encoded=source.next_packet().unwrap().unwrap();
    let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
    let raw=frame.packet.data.clone();
    (source.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap(),raw)
}
#[test]
fn rounded_padding_metadata_removes_the_known_sample_count_without_losing_a_neighbor() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-padding-rounding");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let opus=packet(std::fs::read(base.join("opus-control.mkv")).unwrap()).0.data;
    assert_eq!(opus.len(),960*4);assert!(opus.chunks_exact(4).all(|b|f32::from_le_bytes(b.try_into().unwrap()).is_finite()));
    let mut wrong=Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()),32<<20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
        let trim=case["trim"].as_u64().unwrap() as usize;let head=case["negative"].as_bool().unwrap();let rate=case["rate"].as_u64().unwrap();
        let expected=if case["opus"].as_bool().unwrap(){if head{opus[trim*4..].to_vec()}else{opus[..opus.len()-trim*4].to_vec()}}else{std::fs::read(base.join(format!("{name}-expected.f32le"))).unwrap()};
        let mut owned=Vec::new();
        if case["opus"].as_bool().unwrap(){
            fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(Cursor::new(bytes.clone()),&mut owned,None,&Default::default()).unwrap();
        }else{
            fvid_media::owned_matroska_pcm::decode_matroska_pcm(Cursor::new(bytes.clone()),&mut owned,None,&Default::default()).unwrap();
        }
        // The PCM interval API renders from container time zero, whereas the
        // packet API below returns only the audible window at its timestamp.
        let mut owned_expected=if head{vec![0;trim*4]}else{Vec::new()};
        owned_expected.extend_from_slice(&expected);
        assert_eq!(owned.len(),owned_expected.len(),"{name} owned container timeline length");
        for (sample,(actual,wanted)) in owned.chunks_exact(4).zip(owned_expected.chunks_exact(4)).enumerate(){
            assert_eq!(actual,wanted,"{name} owned container sample {sample}");
        }
        let (actual,raw)=packet(bytes.clone());
        if case["opus"].as_bool().unwrap(){assert_eq!(raw,opus,"{name} untrimmed control");}
        else{assert_eq!(raw,std::fs::read(base.join(format!("{name}-source.f32le"))).unwrap(),"{name} untrimmed control");}
        if actual.data!=expected {wrong.push(format!("{name}: {} instead of{} samples",actual.data.len()/4,expected.len()/4));continue;}
        let at=Duration::from_nanos(if head{trim as u64*1_000_000_000/rate}else{0});assert_eq!(actual.presentation_time(),at,"{name}");
        let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
        capture::worker_check(Box::new(source),&[(at,expected)]);
    }
    assert!(wrong.is_empty(),"ns quantization discarded a nonpadding neighbor sample: {wrong:?}");
}

#[test]
fn nearest_sample_rounding_does_not_admit_padding_beyond_the_entire_decoded_packet() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-padding-rounding");
    for name in ["oversized-tail","oversized-head"] {
        let bytes=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()),32<<20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap();
        let mut decoder=source.make_decoder().unwrap();let encoded=source.next_packet().unwrap().unwrap();
        let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
        assert_eq!(frame.packet.data.len(),960*4);
        let error=source.present_decoded(frame.packet,frame.source_pts).err().expect("rounding admitted padding greater than the whole packet");
        assert!(error.to_string().contains("discard padding exceeds decoded packet"),"{error}");
        let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
        capture::worker_refusal(Box::new(source),"discard padding exceeds decoded packet");
    }
}

#[test]
fn vorbis_declared_endpoint_uses_the_same_quantized_padding_as_rendered_samples() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-padding-rounding");
    for (name,frames) in [("vorbis-control",448),("vorbis-two-sample-tail",446)] {
        let bytes=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()),32<<20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap();
        let mut decoder=source.make_decoder().unwrap();let mut rendered=0;let mut intervals=Vec::new();
        while let Some(encoded)=source.next_packet().unwrap() {
            if let Some(frame)=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap() {
                if let Some(packet)=source.present_decoded(frame.packet,frame.source_pts).unwrap(){
                    assert!(packet.data.chunks_exact(4).all(|b|f32::from_le_bytes(b.try_into().unwrap())==0.0));
                    let index=intervals.len();let starts=[0,80,160,240,368];
                    assert_eq!(packet.presentation_time(),Duration::from_nanos(starts[index]*1_000_000_000/48000));
                    let count=if index==4 {frames-368}else{[80,80,80,128][index]};
                    assert_eq!(packet.data.len()/8,count);
                    rendered+=packet.data.len()/8;intervals.push((packet.presentation_time(),packet.data));
                }
            }
        }
        assert_eq!(rendered,frames,"{name} native rendered samples");
        assert_eq!(source.duration(),Some(Duration::from_nanos(frames as u64*1_000_000_000/48000)),"{name} declared endpoint");
        let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();
        capture::worker_check(Box::new(source),&intervals);
    }
}
