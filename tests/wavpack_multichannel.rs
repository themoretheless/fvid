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
            .join("tests/fixtures/playback-errors/wavpack-multichannel")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn pcm(d: &mut dyn AudioDecode, raw: &[u8], pts: u64) -> Vec<u8> {
    d.decode_encoded(raw, pts, 0).unwrap().unwrap().data
}
#[test]
fn complete_frames_interleave_every_channel_and_match_independent_component_controls() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 115);
    for c in m["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap();
        let n = (rate / 50) as usize;
        let oracle = read(c["oracle"].as_str().unwrap());
        let mut d = WavpackDecoder::new(channels as u16).unwrap();
        for _ in 0..2 {
            let mut output = Vec::new();
            for (i, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                let actual = d
                    .decode_encoded(
                        &read(f["file"].as_str().unwrap()),
                        f["pts"].as_u64().unwrap(),
                        0,
                    )
                    .unwrap_or_else(|e| panic!("{} frame{i}: {e}", c["name"]))
                    .unwrap();
                assert_eq!(actual.pts, f["pts"].as_u64().unwrap());
                assert_eq!(actual.timebase_den, rate as u32);
                assert_eq!(actual.data.len(), n * channels * 4);
                assert!(
                    actual.data == oracle[i * n * channels * 4..(i + 1) * n * channels * 4],
                    "{} frame{i} scalar",
                    c["name"]
                );
                let mut offset = 0;
                for component in f["components"].as_array().unwrap() {
                    let width = component["channels"].as_u64().unwrap() as usize;
                    let mut ctl = WavpackDecoder::new(width as u16).unwrap();
                    let control = pcm(&mut ctl, &read(component["file"].as_str().unwrap()), 0);
                    for row in 0..n {
                        let at = (row * channels + offset) * 4;
                        assert!(
                            actual.data[at..at + width * 4]
                                == control[row * width * 4..(row + 1) * width * 4],
                            "{} channel{offset}",
                            c["name"]
                        );
                    }
                    offset += width;
                }
                assert_eq!(offset, channels);
                output.extend(actual.data);
            }
            assert!(output == oracle, "{} total", c["name"]);
            d.reset();
        }
    }
}
fn drain(
    stream: &mut fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>>,
    decoder: &mut dyn AudioDecode,
) -> Vec<u8> {
    let mut all = Vec::new();
    while let Some(source) = stream.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&source.data, source.pts, source.duration.max(0) as u64)
            .unwrap()
            .unwrap();
        if let Some(packet) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            assert_eq!(packet.timebase_den, 1_000_000_000);
            all.extend(packet.data);
        }
    }
    all
}
#[test]
fn matroska_multichannel_mapping_preserves_layout_samples_and_partial_block_seek() {
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = read(c["file"].as_str().unwrap());
        let oracle = read(c["oracle"].as_str().unwrap());
        let channels = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut count = 0;
        while let Some(f) = video.read_frame_raw().unwrap() {
            assert_eq!(f.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            count += 1;
        }
        assert_eq!(count, 3);
        let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(stream.channels() as usize, channels);
        assert_eq!(
            stream.speaker_mask().unwrap(),
            Some(c["mask"].as_u64().unwrap() as u32)
        );
        let mut decoder = stream.make_decoder().unwrap();
        assert!(
            drain(&mut stream, &mut *decoder) == oracle,
            "{} initial",
            c["name"]
        );
        stream.rewind();
        decoder.reset();
        assert!(
            drain(&mut stream, &mut *decoder) == oracle,
            "{} rewind",
            c["name"]
        );
        stream.seek_to(35_000_000);
        decoder.reset();
        let first = (rate * 35).div_ceil(1000) as usize;
        assert!(
            drain(&mut stream, &mut *decoder) == oracle[first * channels * 4..],
            "{} seek",
            c["name"]
        );
        let router = stream.stereo_router().unwrap().unwrap();
        assert_eq!(router.input_channels() as usize, channels);
        let row: Vec<f32> = oracle[..channels * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let routed = router.route(&oracle[..channels * 4], 8).unwrap();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let expected = if c["mask"] == 0 {
            let average = row.iter().sum::<f32>() / channels as f32;
            [average, average]
        } else if channels == 3 {
            [row[0] + row[2] * k, row[1] + row[2] * k]
        } else {
            let mut pair = [
                row[0] + row[2] * k + row[4] * k,
                row[1] + row[2] * k + row[5] * k,
            ];
            if channels == 8 {
                pair[0] += row[6] * k;
                pair[1] += row[7] * k;
            }
            pair
        };
        for i in 0..2 {
            assert!(
                (routed[i] - expected[i]).abs() < 0.000001,
                "{} speaker fold",
                c["name"]
            );
        }
    }
}
#[test]
fn component_errors_refuse_whole_frames_without_replacing_decoder_layout() {
    let m = manifest();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "c6-paired-16-new1")
        .unwrap();
    let seed = read(c["frames"][0]["file"].as_str().unwrap());
    for bad in m["invalid"].as_array().unwrap() {
        let mut d = WavpackDecoder::new(6).unwrap();
        let expected = pcm(&mut d, &seed, 0);
        let error = d
            .decode_encoded(&read(bad["file"].as_str().unwrap()), 0, 0)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(bad["message"].as_str().unwrap()),
            "{}: {error}",
            bad["file"]
        );
        assert_eq!(pcm(&mut d, &seed, 0), expected);
        d.reset();
        assert_eq!(pcm(&mut d, &seed, 0), expected);
        let opened = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(bad["movie"].as_str().unwrap())),
            Default::default(),
        );
        if bad["raw_only"] == true {
            let mut stream = opened.unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            assert!(
                drain(&mut stream, &mut *decoder) == read(c["oracle"].as_str().unwrap()),
                "raw-only fields must not leak into compact mapping"
            );
        } else {
            let error = match opened {
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
            assert!(
                error.contains(bad["message"].as_str().unwrap()),
                "{}: {error}",
                bad["movie"]
            );
        }
    }
    for bad in m["mapping_invalid"].as_array().unwrap() {
        let private: Vec<u8> = bad["private"]
            .as_array()
            .map(|v| v.iter().map(|n| n.as_u64().unwrap() as u8).collect())
            .unwrap_or_else(|| vec![0x10, 4]);
        let error = WavpackDecoder::for_matroska(&private, 8000, 6)
            .and_then(|mut d| d.decode_encoded(&read(bad["file"].as_str().unwrap()), 0, 0))
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(bad["message"].as_str().unwrap()),
            "{}: {error}",
            bad["file"]
        );
        let opened = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(bad["movie"].as_str().unwrap())),
            Default::default(),
        );
        let error = match opened {
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
        assert!(
            error.contains(bad["message"].as_str().unwrap()),
            "{}: {error}",
            bad["movie"]
        );
    }
}
#[test]
fn container_layout_remains_bound_across_decoder_reset_and_direct_seek() {
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("layout-change.mkv")),
        Default::default(),
    )
    .unwrap();
    let mut d = stream.make_decoder().unwrap();
    let p = stream.next_packet().unwrap().unwrap();
    assert!(d.decode_encoded(&p.data, 0, 0).unwrap().is_some());
    let p = stream.next_packet().unwrap().unwrap();
    assert!(
        d.decode_encoded(&p.data, 0, 0)
            .err()
            .unwrap()
            .to_string()
            .contains("layout changed")
    );
    stream.seek_to(20_000_000);
    d.reset();
    let p = stream.next_packet().unwrap().unwrap();
    assert!(
        d.decode_encoded(&p.data, 0, 0)
            .err()
            .unwrap()
            .to_string()
            .contains("layout changed")
    );
    stream.rewind();
    d.reset();
    let p = stream.next_packet().unwrap().unwrap();
    assert!(d.decode_encoded(&p.data, 0, 0).unwrap().is_some());
}
#[test]
fn multichannel_audio_worker_preserves_pcm_and_seek_windows() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    for name in [
        "c6-paired-16-new1",
        "c8-false-float-new1",
        "c6-mono-optimized-new1",
        "c4096-paired-16-new1",
    ] {
        let c = cases.iter().find(|c| c["name"] == name).unwrap();
        let rate = c["rate"].as_u64().unwrap();
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
fn failed_last_component_does_not_commit_a_new_layout() {
    let mut d = WavpackDecoder::new(6).unwrap();
    let error = d
        .decode_encoded(&read("uncommitted-mask.wv"), 0, 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("checksum"), "{error}");
    let good = read("c6-paired-16-new1-0.wv");
    assert!(d.decode_encoded(&good, 0, 0).unwrap().is_some());
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("uncommitted-mask.mkv")),
        Default::default(),
    )
    .unwrap();
    let packet = stream.next_packet().unwrap().unwrap();
    let error = stream
        .make_decoder()
        .unwrap()
        .decode_encoded(&packet.data, 0, 0)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("checksum"), "{error}");
}
