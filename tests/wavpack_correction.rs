#![cfg(feature = "player")]
use fvid::{
    audio::{AudioDecode, AudioStream},
    codec::wavpack_decoder::WavpackDecoder,
};
use std::io::Cursor;
#[path = "support/audio_capture.rs"]
mod audio_capture;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/wavpack-correction")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
#[test]
fn correction_pairs_restore_exact_pcm_through_raw_container_seek_and_presentation() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 306);
    for c in m["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as u16;
        let rate = 8000;
        let expected = read(c["oracle"].as_str().unwrap());
        let frames = c["frames"].as_array().unwrap();
        let mut decoder = WavpackDecoder::new(channels).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for f in frames {
                let p = decoder
                    .decode_with_correction(
                        &read(f["file"].as_str().unwrap()),
                        &read(f["correction"].as_str().unwrap()),
                        f["pts"].as_u64().unwrap(),
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
        let mut compact_decoder = WavpackDecoder::for_matroska(&[0x10, 4], 8000, channels).unwrap();
        let mut compact_pcm = Vec::new();
        for f in frames {
            compact_pcm.extend(
                compact_decoder
                    .decode_with_correction(
                        &read(f["packet"].as_str().unwrap()),
                        &read(f["additional"].as_str().unwrap()),
                        f["pts"].as_u64().unwrap(),
                    )
                    .unwrap()
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(compact_pcm, expected, "{} direct compact pair", c["name"]);
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
        stream.seek_to(35_000_000);
        decoder.reset();
        let mut after = Vec::new();
        while let Some(p) = stream.next_packet().unwrap() {
            let decoded = decoder
                .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                .unwrap()
                .unwrap();
            if let Some(presented) = stream
                .present_decoded(decoded.packet, decoded.source_pts)
                .unwrap()
            {
                after.extend(presented.data);
            }
        }
        assert_eq!(
            after,
            expected[280 * usize::from(channels) * 4..],
            "{} seek",
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
fn main_streams_remain_independently_decodable_with_lossy_histories() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = WavpackDecoder::new(c["channels"].as_u64().unwrap() as u16).unwrap();
        let mut actual = Vec::new();
        for f in c["frames"].as_array().unwrap() {
            actual.extend(
                d.decode_encoded(&read(f["file"].as_str().unwrap()), 0, 0)
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(actual, read(c["lossy"].as_str().unwrap()), "{}", c["name"]);
    }
}

#[test]
fn correction_failures_are_atomic_and_do_not_fall_back_to_lossy_pcm() {
    let m = manifest();
    let base = &m["cases"][0];
    let main = read(base["frames"][0]["file"].as_str().unwrap());
    let good = read(base["frames"][0]["correction"].as_str().unwrap());
    for c in m["invalid"].as_array().unwrap() {
        let mut decoder = WavpackDecoder::new(1).unwrap();
        let message = c["message"].as_str().unwrap();
        let error = decoder
            .decode_with_correction(&main, &read(c["file"].as_str().unwrap()), 0)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(message), "{}: {error}", c["file"]);
        assert_eq!(
            decoder
                .decode_with_correction(&main, &good, 0)
                .unwrap()
                .unwrap()
                .data,
            read(base["oracle"].as_str().unwrap())[..160 * 4]
        );
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
    let error = WavpackDecoder::new(1)
        .unwrap()
        .decode_with_correction(&main, &read("mismatch-window.wvc"), 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("header disagrees"), "{error}");
    for (file, message) in [
        (
            "compact-short.mkv",
            "truncated Matroska WavPack correction header",
        ),
        ("duplicate-id.mkv", "duplicate Matroska BlockAddID"),
        (
            "laced-correction.mkv",
            "laced Matroska WavPack correction is not supported",
        ),
    ] {
        let error = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(file)),
            Default::default(),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains(message), "{file}: {error}");
    }
}
#[test]
fn additions_are_selected_by_id_and_share_the_packet_budget() {
    let m = manifest();
    let base = &m["cases"][0];
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("ignored-id.mkv")),
        Default::default(),
    )
    .unwrap();
    let mut decoder = stream.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        pcm.extend(decoder.decode_encoded(&p.data, 0, 0).unwrap().unwrap().data);
    }
    assert_eq!(pcm, read(base["lossy"].as_str().unwrap()));
    let frame = &base["frames"][0];
    let main = read(frame["packet"].as_str().unwrap());
    let extra = read(frame["additional"].as_str().unwrap());
    let mut reader = fvid::container::webm::WebmReader::open(
        Cursor::new(read(base["file"].as_str().unwrap())),
        Default::default(),
    )
    .unwrap();
    reader.restrict_packet_bytes(main.len() + extra.len() - 1);
    let index = reader.packets.iter().position(|p| p.track == 2).unwrap();
    assert_eq!(reader.read_packet(index).unwrap(), main);
    let error = reader
        .read_block_additional(index, 1)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("packet and additions exceed budget"),
        "{error}"
    );
}

#[test]
fn corrected_audio_worker_preserves_pcm_and_seek_windows() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    for name in [
        "16-stereo-bitrate-delta-new-cross1",
        "24-mono-snr-delta-cross0",
        "32-false-zero-limit-new-cross0",
        "multichannel-bitrate-delta-delta-cross1",
    ] {
        let c = cases
            .iter()
            .find(|c| c["name"].as_str().unwrap().starts_with(name))
            .unwrap();
        let rate = 8000u64;
        let channels = c["channels"].as_u64().unwrap() as usize;
        let n = (rate / 50) as usize;
        let oracle = read(c["oracle"].as_str().unwrap());
        let stride = channels * 4;
        let expected: Vec<_> = (0..3)
            .map(|i| {
                (
                    std::time::Duration::from_millis(i as u64 * 20),
                    oracle[i * n * stride..(i + 1) * n * stride].to_vec(),
                )
            })
            .collect();
        let skip = (rate * 15).div_ceil(1000) as usize;
        let after = vec![
            (
                std::time::Duration::from_nanos(20_000_000 + skip as u64 * 1_000_000_000 / rate),
                oracle[(n + skip) * stride..2 * n * stride].to_vec(),
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

#[test]
fn failed_last_corrected_component_does_not_commit_layout() {
    let m = manifest();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == m["atomic_case"])
        .unwrap();
    let mut decoder = WavpackDecoder::new(6).unwrap();
    let error = decoder
        .decode_with_correction(&read("atomic-mask.wv"), &read("atomic-last-crc.wvc"), 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("checksum"), "{error}");
    let f = &c["frames"][0];
    let pcm = decoder
        .decode_with_correction(
            &read(f["file"].as_str().unwrap()),
            &read(f["correction"].as_str().unwrap()),
            0,
        )
        .unwrap()
        .unwrap();
    assert_eq!(pcm.data, read(c["oracle"].as_str().unwrap())[..160 * 6 * 4]);
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("atomic-mask.mkv")),
        Default::default(),
    )
    .unwrap();
    let p = stream.next_packet().unwrap().unwrap();
    let error = stream
        .make_decoder()
        .unwrap()
        .decode_encoded(&p.data, 0, 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("checksum"), "{error}");
}
