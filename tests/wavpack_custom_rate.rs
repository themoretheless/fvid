#![cfg(feature = "player")]
use fvid::{
    audio::{AudioDecode, AudioStream},
    codec::wavpack_decoder::WavpackDecoder,
};
use std::io::Cursor;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/wavpack-custom-rate")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
#[test]
fn extended_rates_preserve_pcm_and_timebase_in_raw_and_matroska() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 30);
    for c in m["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as u16;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let expected = read(c["oracle"].as_str().unwrap());
        let frames = c["frames"].as_array().unwrap();
        let mut decoder = WavpackDecoder::new(channels).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for f in frames {
                let p = decoder
                    .decode_encoded(
                        &read(f["file"].as_str().unwrap()),
                        f["pts"].as_u64().unwrap(),
                        0,
                    )
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
                    .unwrap();
                assert_eq!(p.timebase_den, rate);
                assert_eq!(p.pts, f["pts"].as_u64().unwrap());
                actual.extend(p.data);
            }
            assert_eq!(actual, expected, "{} raw", c["name"]);
            decoder.reset();
        }
        let bytes = read(c["file"].as_str().unwrap());
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(stream.sample_rate(), rate);
        assert_eq!(stream.channels(), channels);
        let mut decoder = stream.make_decoder().unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(p) = stream.next_packet().unwrap() {
                let decoded = decoder.decode_encoded(&p.data, 0, 0).unwrap().unwrap();
                assert_eq!(decoded.timebase_den, rate);
                actual.extend(decoded.data);
                count += 1;
            }
            assert_eq!(count, 3);
            assert_eq!(actual, expected, "{} compact", c["name"]);
            stream.rewind();
            decoder.reset();
        }
        // Exercise presentation clocks too, rather than only direct packet decoding.
        let mut actual = Vec::new();
        while let Some(p) = stream.next_packet().unwrap() {
            let decoded = decoder
                .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                .unwrap()
                .unwrap();
            let presented = stream
                .present_decoded(decoded.packet, decoded.source_pts)
                .unwrap()
                .unwrap();
            assert_eq!(presented.timebase_den, 1_000_000_000);
            actual.extend(presented.data);
        }
        assert_eq!(actual, expected, "{} presentation", c["name"]);
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes), 32 << 20).unwrap();
        let mut count = 0;
        while let Some(f) = video.read_frame_raw().unwrap() {
            assert_eq!(f.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
#[test]
fn invalid_rates_refuse_specific_metadata_without_poisoning_decoder() {
    let m = manifest();
    let good = read(m["cases"][0]["frames"][0]["file"].as_str().unwrap());
    for c in m["invalid"].as_array().unwrap() {
        let message = c["message"].as_str().unwrap();
        let mut d = WavpackDecoder::new(1).unwrap();
        let error = d
            .decode_encoded(&read(c["file"].as_str().unwrap()), 0, 0)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(message), "{}: {error}", c["file"]);
        let p = d.decode_encoded(&good, 0, 0).unwrap().unwrap();
        assert_eq!(p.timebase_den, 37500);
        assert_eq!(p.data.len(), 750 * 4);
        let error = match fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(c["movie"].as_str().unwrap())),
            Default::default(),
        ) {
            Err(e) => e.to_string(),
            Ok(mut stream) => {
                let p = stream.next_packet().unwrap().unwrap();
                stream
                    .make_decoder()
                    .unwrap()
                    .decode_encoded(&p.data, 0, 0)
                    .err()
                    .unwrap()
                    .to_string()
            }
        };
        assert!(error.contains(message), "{}: {error}", c["movie"]);
    }
}
