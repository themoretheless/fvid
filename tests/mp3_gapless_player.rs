#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod audio_capture;
use fvid::audio::{AudioDecode, AudioStream};
use std::{path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp3-gapless")
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    m["cases"].as_array().unwrap().clone()
}
fn decoded(
    stream: &mut dyn AudioStream,
    decoder: &mut dyn AudioDecode,
) -> Vec<(Duration, Vec<u8>)> {
    let mut packets = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(pcm) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            packets.push((pcm.presentation_time(), pcm.data));
        }
    }
    packets
}
fn flatten(p: &[(Duration, Vec<u8>)]) -> Vec<u8> {
    p.iter().flat_map(|(_, b)| b.iter().copied()).collect()
}
#[test]
fn raw_gapless_windows_duration_seek_reset_and_metadata_refusals_preserve_pcm() {
    let cases = cases();
    assert_eq!(cases.len(), 115);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap() as u32;
        let stride = case["channels"].as_u64().unwrap() as usize * 4;
        let start = case["start"].as_u64().unwrap() as usize;
        let end = case["end"].as_u64().unwrap() as usize;
        let bytes = std::fs::read(root().join(format!("{name}.mp3"))).unwrap();
        let reference = std::fs::read(
            root()
                .parent()
                .unwrap()
                .join("mp3-decoder")
                .join(format!("{}.f32le", case["source"].as_str().unwrap())),
        )
        .unwrap();
        let expected = &reference[start * stride..end * stride];
        let mut reader =
            fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(
            reader.mp3().samples(),
            case["coded"].as_u64().unwrap(),
            "{name} coded count"
        );
        assert_eq!(
            reader.mp3().presentation_samples(),
            (end - start) as u64,
            "{name} audible count"
        );
        let duration =
            Duration::from_nanos(((end - start) as u128 * 1_000_000_000 / u128::from(rate)) as u64);
        assert_eq!(reader.duration(), Some(duration));
        let mut decoder = reader
            .make_decoder()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let baseline = decoded(&mut reader, decoder.as_mut());
        assert_eq!(flatten(&baseline), expected, "{name}");
        let mut at = 0;
        for (stamp, bytes) in &baseline {
            assert_eq!(
                stamp.as_nanos(),
                at as u128 * 1_000_000_000 / u128::from(rate)
            );
            at += bytes.len() / stride;
        }
        for target in [
            end - start,
            1,
            (end - start) / 2,
            0,
            end - start - 1,
            end - start + 17,
        ] {
            assert_eq!(
                reader.seek_to(target as i64),
                target.min(end - start) as i64
            );
            decoder.reset();
            let actual = decoded(&mut reader, decoder.as_mut());
            assert_eq!(
                flatten(&actual),
                expected[target.min(end - start) * stride..],
                "{name} seek {target}"
            );
        }
        reader.rewind();
        decoder.reset();
        assert_eq!(
            decoded(&mut reader, decoder.as_mut()),
            baseline,
            "{name} rewind"
        );
    }
}
#[test]
fn info_history_is_required_and_is_retained_across_resets() {
    for name in ["primed-history", "primed-free"] {
        let bytes = std::fs::read(root().join(format!("{name}.mp3"))).unwrap();
        let first_audio = &bytes[960..1920];
        let error = fvid_codecs::mp3::Decoder::default()
            .decode(first_audio)
            .err()
            .unwrap();
        assert!(error.to_string().contains("history missing"), "{error}");
        let mut reader =
            fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        let mut decoder = reader.make_decoder().unwrap();
        let first = reader.next_packet().unwrap().unwrap();
        assert!(
            decoder
                .decode_encoded(&first.data, 0, first.duration as u64)
                .unwrap()
                .is_some()
        );
        decoder.reset();
        assert!(
            decoder
                .decode_encoded(&first.data, 0, first.duration as u64)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn gapless_raw_and_matroska_workers_keep_partial_packets_after_seek() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap() as u32;
        for raw in [true, false] {
            if !raw && case["no_video"].as_bool() == Some(true) {
                continue;
            }
            let bytes =
                std::fs::read(root().join(format!("{name}.{}", if raw { "mp3" } else { "mkv" })))
                    .unwrap();
            let mut stream: Box<dyn AudioStream> = if raw {
                Box::new(
                    fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default())
                        .unwrap(),
                )
            } else {
                {
                    let mut video = fvid::playback_webm::WebmVideoReader::open_software(
                        std::io::Cursor::new(&bytes),
                        32 << 20,
                    )
                    .unwrap();
                    let mut count = 0;
                    while video.read_frame().unwrap() {
                        assert_eq!(video.dimensions(), [16, 16]);
                        assert!(video.rgb().iter().all(|&v| v == 130));
                        count += 1;
                    }
                    assert_eq!(count, case["video_frames"].as_u64().unwrap_or(4));
                }
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        std::io::Cursor::new(bytes),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            let mut decoder = stream.make_decoder().unwrap();
            let baseline = decoded(stream.as_mut(), decoder.as_mut());
            let source = std::fs::read(
                root()
                    .parent()
                    .unwrap()
                    .join("mp3-decoder")
                    .join(format!("{}.f32le", case["source"].as_str().unwrap())),
            )
            .unwrap();
            let stride = usize::from(stream.channels()) * 4;
            let start = case["start"].as_u64().unwrap() as usize;
            let end = case["end"].as_u64().unwrap() as usize;
            assert_eq!(
                flatten(&baseline),
                source[start * stride..end * stride],
                "{name} raw {raw}"
            );
            let target = if raw {
                i64::from(rate) / 25
            } else {
                40_000_000
            };
            stream.seek_to(target);
            decoder.reset();
            let suffix = decoded(stream.as_mut(), decoder.as_mut());
            assert_eq!(
                flatten(&suffix),
                source[(start + rate as usize / 25) * stride..end * stride],
                "{name} raw {raw} seek"
            );
            stream.rewind();
            audio_capture::worker_check_with_seek_expected(
                stream,
                &baseline,
                Duration::from_millis(40),
                &suffix,
            );
        }
    }
}
#[test]
fn coded_info_marker_remains_audio_and_header_only_file_has_no_audio() {
    let bytes = std::fs::read(root().join("coded-marker.mp3")).unwrap();
    assert!(!fvid_codecs::mp3::is_info_frame(&bytes[..960]).unwrap());
    let mut stream =
        fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
    assert!(!stream.mp3().info_frame);
    assert_eq!(stream.mp3().packets(), 4);
    let mut owned = stream.make_decoder().unwrap();
    let mut reference = fvid::codec::symphonia_decoder::SymphoniaDecoder::new(
        "coded marker reference",
        symphonia::core::codecs::audio::well_known::CODEC_ID_MP3,
        &[],
        48000,
        1,
    )
    .unwrap();
    let expected = std::fs::read(root().join("coded-marker.f32le")).unwrap();
    let mut count = 0;
    let mut peak = 0f32;
    while let Some(p) = stream.next_packet().unwrap() {
        let a = owned
            .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
            .unwrap()
            .unwrap();
        let b = reference
            .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(a.data.len(), b.data.len());
        // Symphonia does not preserve mixed start/stop lower-band windows;
        // compare its first normal frame only, and all frames to the independent
        // absolute-timeline/convolution PCM oracle.
        if count == 0 {
            for (a, b) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 5e-5);
            }
        }
        let golden = &expected[count * 1152 * 4..(count + 1) * 1152 * 4];
        for (a, b) in a.data.chunks_exact(4).zip(golden.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            peak = peak.max((a - b).abs());
            assert!(
                (a - b).abs() < 5e-5,
                "frame={count}, owned={a}, reference={b}, error={}",
                (a - b).abs()
            );
        }
        count += 1;
    }
    assert_eq!(count, 4);
    assert!(peak.is_finite());
    let video = std::fs::read(root().join("coded-marker.mkv")).unwrap();
    let mut video =
        fvid::playback_webm::WebmVideoReader::open_software(std::io::Cursor::new(video), 32 << 20)
            .unwrap();
    let mut count = 0;
    while video.read_frame().unwrap() {
        count += 1;
    }
    assert_eq!(count, 4);
    let video = std::fs::read(root().join("coded-marker.mkv")).unwrap();
    let mut audio = fvid::playback_webm_audio::WebmAudioReader::open(
        std::io::Cursor::new(video),
        Default::default(),
    )
    .unwrap();
    let mut decoder = audio.make_decoder().unwrap();
    let actual = flatten(&decoded(&mut audio, decoder.as_mut()));
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(
            (a - b).abs() < 5e-5,
            "Matroska marker PCM error {}",
            (a - b).abs()
        );
    }
    let header = std::fs::read(root().join("info-only.mp3")).unwrap();
    let error = fvid::playback_mp3::Mp3AudioReader::open(header.as_slice(), Default::default())
        .err()
        .unwrap();
    assert!(error.to_string().contains("nothing but an info frame"));
}
