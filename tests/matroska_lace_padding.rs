#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
#[test]
fn signed_block_padding_spans_fixed_xiph_and_ebml_laces_without_refusing_valid_audio() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-lace-padding");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let source_pcm=std::fs::read(base.join("expected-source.f32le")).unwrap();
    let mut refused=Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();let media=std::fs::read(base.join(name)).unwrap();
        let expected=std::fs::read(base.join(case["oracle"].as_str().unwrap())).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(media.clone()),32<<20).unwrap();
        for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(media.clone()),Default::default()).unwrap();
        let mut decoder=source.make_decoder().unwrap();let mut actual=Vec::new();let mut intervals=Vec::new();let mut error=None;
        for index in 0..3 {
            let encoded=source.next_packet().unwrap().unwrap();assert_eq!(encoded.pts,index as i64*20_000_000);
            let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
            assert_eq!(frame.packet.data,&source_pcm[index*640..(index+1)*640]);
            match source.present_decoded(frame.packet,frame.source_pts) {
                Ok(Some(packet))=>{intervals.push((packet.presentation_time(),packet.data.clone()));actual.extend(packet.data);}
                Ok(None)=>{},Err(e)=>error=Some(e.to_string()),
            }
        }
        assert!(source.next_packet().unwrap().is_none());
        if let Some(error)=error {refused.push(format!("{name}: {error}"));continue;}
        assert_eq!(actual,expected,"{name}");
        let wanted:Vec<_>=case["packets"].as_array().unwrap().iter().map(|p|{
            let start=p["first"].as_u64().unwrap() as usize*4;let end=start+p["frames"].as_u64().unwrap() as usize*4;
            (Duration::from_nanos(p["pts_ns"].as_u64().unwrap()),source_pcm[start..end].to_vec())
        }).collect();assert_eq!(intervals,wanted,"{name}");
        let source=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(media),Default::default()).unwrap();
        let end=wanted.last().map_or(Duration::ZERO,|(at,data)|*at+Duration::from_nanos(data.len() as u64/4*125000));
        assert_eq!(source.duration(),Some(end),"{name} declared audio endpoint");
        capture::worker_check_with_seek(Box::new(source),&wanted,Some(Duration::from_millis(20)));
    }
    assert!(refused.is_empty(),"valid block padding was applied to only one lace: {refused:?}");
}

#[test]
fn owned_pcm_export_keeps_one_leading_padding_origin_across_laced_frames() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-lace-padding");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong=Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();let media=std::fs::read(base.join(name)).unwrap();
        let expected=std::fs::read(base.join(case["oracle"].as_str().unwrap())).unwrap();
        for precise in [false,true] {
            let mut output=Vec::new();
            let result=if precise {
                fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(Cursor::new(media.clone()),&mut output,None,&Default::default())
            }else{
                fvid_media::owned_matroska_pcm::decode_matroska_pcm(Cursor::new(media.clone()),&mut output,None,&Default::default())
            };
            if expected.is_empty(){assert!(result.err().unwrap().to_string().contains("contains no samples"));continue;}
            result.unwrap();
            let wanted=if precise {expected.chunks_exact(4).flat_map(|b|f64::from(f32::from_le_bytes(b.try_into().unwrap())).to_le_bytes()).collect::<Vec<_>>()}else{expected.clone()};
            if output!=wanted {wrong.push(format!("{name},f64={precise}: {} instead of{} bytes",output.len(),wanted.len()));}
        }
    }
    assert!(wrong.is_empty(),"laced leading padding was reinserted as output silence: {wrong:?}");
}

#[test]
fn padding_larger_than_the_whole_laced_block_and_packet_limits_refuse_precisely() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-lace-padding");
    for name in ["too-large.mkv","min-negative.mkv"] {
        let bytes=std::fs::read(base.join(name)).unwrap();
        let error=fvid::container::webm::WebmReader::open(Cursor::new(bytes.clone()),Default::default()).err().expect("oversized block padding accepted");
        assert!(error.to_string().contains("discard padding exceeds laced block duration"),"{name}: {error}");
        let error=fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes),Default::default()).err().expect("owned oversized block padding accepted");
        assert!(error.to_string().contains("discard padding exceeds laced block duration"),"{name}: {error}");
    }
    let bytes=std::fs::read(base.join("fixed-control.mkv")).unwrap();
    let error=fvid::container::webm::WebmReader::open(Cursor::new(bytes),fvid::container::webm::Limits{packet_bytes:319,..Default::default()}).err().expect("lace packet budget ignored");
    assert!(error.to_string().contains("packet exceeds budget"),"{error}");
}
