#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(kind: &str, bytes: Vec<u8>) -> fvid::Result<Box<dyn AudioStream>> {
    let source: Box<dyn AudioStream> = match kind {
        "wav" => Box::new(fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )?),
        "aiff" => Box::new(fvid::playback_aiff::AiffAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )?),
        "au" => Box::new(fvid::playback_au::AuAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )?),
        _ => unreachable!(),
    };
    Ok(source)
}
fn read(mut source: Box<dyn AudioStream>) -> (Vec<u8>, Vec<(std::time::Duration, Vec<u8>)>) {
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    let mut intervals = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        assert!(
            frame
                .packet
                .data
                .chunks_exact(4)
                .all(|s| f32::from_le_bytes(s.try_into().unwrap()).is_finite())
        );
        pcm.extend_from_slice(&frame.packet.data);
        intervals.push((frame.packet.presentation_time(), frame.packet.data));
    }
    source.validate_eof().unwrap();
    (pcm, intervals)
}
#[test]
fn fixed_frame_audio_runs_refuse_orphan_bytes_instead_of_hiding_them_from_native_decode() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/pcm-run-alignment");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let kind = case["kind"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let expected = std::fs::read(base.join(case["expected"].as_str().unwrap())).unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let source = open(kind, bytes.clone()).unwrap();
        let end = source.time_of(case["frames"].as_i64().unwrap());
        assert_eq!(source.duration(), Some(end));
        let (actual, intervals) = read(source);
        assert_eq!(actual, expected, "{name} complete numeric native control");
        capture::worker_check_with_seek(open(kind, bytes).unwrap(), &intervals, Some(end));
        for bad in case["bad"].as_array().unwrap() {
            let filename = bad["file"].as_str().unwrap();
            let bytes = std::fs::read(base.join(filename)).unwrap();
            match open(kind, bytes.clone()) {
                Err(error) => assert!(
                    error
                        .to_string()
                        .contains("sample run ends in an incomplete channel frame"),
                    "{filename} unrelated refusal: {error}"
                ),
                Ok(source) => {
                    let (actual, partial) = read(source);
                    assert_eq!(
                        actual, expected,
                        "{filename} orphan concealed before native decode"
                    );
                    capture::worker_check(open(kind, bytes).unwrap(), &partial);
                    wrong.push(format!("{filename}: silently removed {} declared audio bytes, successful native EOF{}frames",bad["tail"],actual.len()/4/case["channels"].as_u64().unwrap() as usize));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "fixed-frame packet rounding concealed malformed audio bytes: {wrong:?}"
    );
}
