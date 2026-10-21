#![cfg(feature = "player")]
use fvid::{
    audio::{AudioDecode, AudioStream},
    codec::wavpack_decoder::WavpackDecoder,
};
use std::io::Cursor;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/wavpack-dsd")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
#[test]
fn dsd_modes_reconstruct_exact_bytes_and_stateful_pcm_in_raw_and_matroska() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 256);
    for c in m["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as u16;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let n = c["samples"].as_u64().unwrap() as usize;
        let expected = read(c["pcm"].as_str().unwrap());
        let frames = c["frames"].as_array().unwrap();
        let mut decoder = WavpackDecoder::new(channels).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for f in frames {
                let (bytes, decoded_rate) = decoder
                    .decode_dsd(&read(f["file"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} raw bytes: {e}", c["name"]));
                assert_eq!(decoded_rate, rate);
                let first = f["pts"].as_u64().unwrap() as usize * usize::from(channels);
                let oracle = read(c["oracle"].as_str().unwrap());
                assert_eq!(
                    bytes,
                    oracle[first..first + n * usize::from(channels)],
                    "{} DSD",
                    c["name"]
                );
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
        let target = ((n * 3 / 2) as u64 * 1_000_000_000).div_ceil(u64::from(rate));
        stream.seek_to(target as i64);
        decoder.reset();
        let mut after = Vec::new();
        while let Some(p) = stream.next_packet().unwrap() {
            let decoded = decoder
                .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                .unwrap()
                .unwrap();
            if let Some(packet) = stream
                .present_decoded(decoded.packet, decoded.source_pts)
                .unwrap()
            {
                after.extend(packet.data);
            }
        }
        let first = (target * u64::from(rate)).div_ceil(1_000_000_000) as usize;
        assert_eq!(
            after,
            expected[first * usize::from(channels) * 4..],
            "{} seek filter history",
            c["name"]
        );
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
fn malformed_dsd_is_atomic_and_does_not_poison_next_frame() {
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
        assert_eq!(p.timebase_den, 44100);
        assert_eq!(p.data.len(), 128 * 4);
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

#[test]
fn failed_dsd_component_preserves_filter_history_and_uncommitted_layout() {
    let m = manifest();
    let case = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == m["atomic_case"])
        .unwrap();
    let frames = case["frames"].as_array().unwrap();
    let expected = read(case["pcm"].as_str().unwrap());
    let mut decoder = WavpackDecoder::new(6).unwrap();
    let error = decoder
        .decode_encoded(&read("uncommitted-mask.wv"), 128, 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("DSD checksum"), "{error}");
    let first = decoder
        .decode_encoded(&read(frames[0]["file"].as_str().unwrap()), 0, 0)
        .unwrap()
        .unwrap();
    assert_eq!(first.data, expected[..128 * 6 * 4]);
    let error = decoder
        .decode_encoded(&read("last-crc.wv"), 128, 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("DSD checksum"), "{error}");
    for i in 1..3 {
        let frame = decoder
            .decode_encoded(
                &read(frames[i]["file"].as_str().unwrap()),
                i as u64 * 128,
                0,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            frame.data,
            expected[i * 128 * 6 * 4..(i + 1) * 128 * 6 * 4],
            "filter advanced after bad DSD component"
        );
    }
}
#[path = "support/audio_capture.rs"]
mod audio_capture;
#[test]
fn dsd_audio_worker_preserves_pcm_seek_and_filter_preroll() {
    let m = manifest();
    for name in [
        "raw-mono-p3-square",
        "fast-stereo-p4-ramp",
        "rle-false-p0-noise",
        "high-six-p3-noise",
    ] {
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let pcm = read(c["pcm"].as_str().unwrap());
        let stride = channels * 4;
        let expected: Vec<_> = (0..3)
            .map(|i| {
                (
                    std::time::Duration::from_nanos(i as u64 * 128 * 1_000_000_000 / rate),
                    pcm[i * 128 * stride..(i + 1) * 128 * stride].to_vec(),
                )
            })
            .collect();
        let target = (192u64 * 1_000_000_000).div_ceil(rate);
        let origin = 128u64 * 1_000_000_000 / rate;
        let skip = ((target - origin) * rate).div_ceil(1_000_000_000) as usize;
        let after = vec![
            (
                std::time::Duration::from_nanos(origin + skip as u64 * 1_000_000_000 / rate),
                pcm[(128 + skip) * stride..256 * stride].to_vec(),
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
            std::time::Duration::from_nanos(target),
            &after,
        );
    }
}
