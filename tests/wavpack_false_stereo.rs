#![cfg(feature = "player")]
use fvid::{
    audio::{AudioDecode, AudioStream},
    codec::wavpack_decoder::WavpackDecoder,
};
use std::io::Cursor;
#[path = "support/audio_capture.rs"]
mod audio_capture;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/wavpack-false-stereo")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
#[test]
fn mono_coded_stereo_matches_literal_pcm_and_mono_controls_after_reset() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 99);
    for c in m["cases"].as_array().unwrap() {
        let oracle = read(c["oracle"].as_str().unwrap());
        let mut d = WavpackDecoder::new(2).unwrap();
        for _ in 0..2 {
            let mut output = Vec::new();
            for f in c["frames"].as_array().unwrap() {
                let pcm = d
                    .decode_encoded(
                        &read(f["file"].as_str().unwrap()),
                        f["pts"].as_u64().unwrap(),
                        0,
                    )
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
                    .unwrap();
                assert_eq!(pcm.pts, f["pts"].as_u64().unwrap());
                assert_eq!(pcm.timebase_den, c["rate"].as_u64().unwrap() as u32);
                if let Some(mono) = f["mono"].as_str() {
                    let mut control = WavpackDecoder::new(1).unwrap();
                    let mono = control.decode_encoded(&read(mono), 0, 0).unwrap().unwrap();
                    let duplicated: Vec<u8> = mono
                        .data
                        .chunks_exact(4)
                        .flat_map(|x| x.iter().chain(x).copied())
                        .collect();
                    assert_eq!(pcm.data, duplicated, "{}", c["name"]);
                }
                output.extend(pcm.data);
            }
            assert_eq!(output, oracle, "{} scalar", c["name"]);
            d.reset();
        }
    }
}
#[test]
fn invalid_channel_flags_and_crc_are_refused_without_poisoning_next_block() {
    let m = manifest();
    let seed = read(m["cases"][0]["frames"][0]["file"].as_str().unwrap());
    for c in m["invalid"].as_array().unwrap() {
        let mut d = WavpackDecoder::new(2).unwrap();
        let error = d
            .decode_encoded(&read(c["file"].as_str().unwrap()), 0, 0)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(c["message"].as_str().unwrap()),
            "{}: {error}",
            c["file"]
        );
        let error = match fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(c["movie"].as_str().unwrap())),
            Default::default(),
        ) {
            Err(error) => error.to_string(),
            Ok(mut stream) => {
                let source = stream.next_packet().unwrap().unwrap();
                stream
                    .make_decoder()
                    .unwrap()
                    .decode_encoded(&source.data, 0, 0)
                    .err()
                    .unwrap()
                    .to_string()
            }
        };
        assert!(
            error.contains(c["message"].as_str().unwrap()),
            "{}: {error}",
            c["movie"]
        );
        assert!(d.decode_encoded(&seed, 0, 0).unwrap().is_some());
        d.reset();
        assert!(d.decode_encoded(&seed, 0, 0).unwrap().is_some());
    }
    assert!(
        WavpackDecoder::new(1)
            .unwrap()
            .decode_encoded(&seed, 0, 0)
            .err()
            .unwrap()
            .to_string()
            .contains("channels disagree")
    );
}
fn drain(
    stream: &mut fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>>,
    decoder: &mut dyn AudioDecode,
) -> Vec<u8> {
    let mut all = Vec::new();
    while let Some(source) = stream.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&source.data, source.pts, source.duration.max(0) as u64)
            .unwrap()
        {
            if let Some(pcm) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                all.extend(pcm.data);
            }
        }
    }
    all
}
#[test]
fn matroska_video_and_audio_decode_rewind_and_trim_seek_without_external_codecs() {
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = read(c["file"].as_str().unwrap());
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 16 << 20)
                .unwrap();
        let mut frames = 0;
        while let Some(p) = video.read_frame_raw().unwrap() {
            assert_eq!(p.into_rgb(16 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            frames += 1;
        }
        assert_eq!(frames, 3);
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(stream.codec(), "A_WAVPACK4");
        assert_eq!(stream.channels(), 2);
        assert_eq!(stream.extra_data(), [0x10, 4]);
        let mut decoder = stream.make_decoder().unwrap();
        let oracle = read(c["oracle"].as_str().unwrap());
        assert_eq!(
            drain(&mut stream, &mut *decoder),
            oracle,
            "{} initial",
            c["name"]
        );
        stream.rewind();
        decoder.reset();
        assert_eq!(
            drain(&mut stream, &mut *decoder),
            oracle,
            "{} rewind",
            c["name"]
        );
        stream.seek_to(35_000_000);
        decoder.reset();
        let first = (c["rate"].as_u64().unwrap() * 35).div_ceil(1000) as usize;
        assert_eq!(
            drain(&mut stream, &mut *decoder),
            oracle[first * 8..],
            "{} seek",
            c["name"]
        );
    }
}
#[test]
fn matroska_mapping_validates_version_short_headers_and_rate() {
    assert!(WavpackDecoder::for_matroska(&[], 8000, 2).is_err());
    assert!(WavpackDecoder::for_matroska(&[0, 5], 8000, 2).is_err());
    let mut d = WavpackDecoder::for_matroska(&[0x10, 4], 8000, 2).unwrap();
    assert!(
        d.decode_encoded(&[0; 11], 0, 0)
            .err()
            .unwrap()
            .to_string()
            .contains("truncated Matroska")
    );
    let m = manifest();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["rate"] != 8000)
        .unwrap();
    let raw = read(c["frames"][0]["file"].as_str().unwrap());
    assert!(
        d.decode_encoded(&raw[20..], 0, 0)
            .err()
            .unwrap()
            .to_string()
            .contains("sample rate disagrees")
    );
}

#[test]
fn audio_worker_preserves_pcm_and_exact_seek_window() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    for c in [
        &cases[0],
        cases
            .iter()
            .find(|c| c["rate"] == 37500 && c["name"].as_str().unwrap().ends_with("sent-dups"))
            .unwrap(),
        cases.last().unwrap(),
    ] {
        let rate = c["rate"].as_u64().unwrap();
        let n = (rate / 50) as usize;
        let oracle = read(c["oracle"].as_str().unwrap());
        let expected: Vec<_> = (0..3)
            .map(|i| {
                (
                    std::time::Duration::from_millis(i as u64 * 20),
                    oracle[i * n * 8..(i + 1) * n * 8].to_vec(),
                )
            })
            .collect();
        let skip = (rate * 15).div_ceil(1000) as usize;
        let after = vec![
            (
                std::time::Duration::from_nanos(20_000_000 + skip as u64 * 1_000_000_000 / rate),
                oracle[(n + skip) * 8..2 * n * 8].to_vec(),
            ),
            expected[2].clone(),
        ];
        let stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        audio_capture::worker_check_with_seek_expected(
            Box::new(stream),
            &expected,
            std::time::Duration::from_millis(35),
            &after,
        );
    }
}
