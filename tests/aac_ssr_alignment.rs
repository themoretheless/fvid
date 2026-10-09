use fvid::container::mp4::Mp4Reader;
use fvid_media::owned_aac::{
    aac_native::NativeAacDecoder,
    aac_ssr_alignment::{LaneInput, OutputGain, SsrPcmAlignment},
};
use serde_json::Value;
use std::io::Cursor;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ssr-alignment.json"
    ))
    .unwrap()
}
fn acceptance_cases() -> Vec<Value> {
    let m = manifest();
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["channels_cases"].as_array().unwrap())
        .chain(m["fixed_cases"].as_array().unwrap())
        .chain(m["tail_cases"].as_array().unwrap())
        .chain(m["arrival_cases"].as_array().unwrap())
        .cloned()
        .collect()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packets(video: &Value) -> Vec<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(video["file"].as_str().unwrap());
    let mut reader = Mp4Reader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Default::default(),
    )
    .unwrap();
    let track = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    (0..reader.tracks()[track].samples.len())
        .map(|i| {
            let mut out = vec![];
            reader.read_packet(track, i, &mut out).unwrap();
            out
        })
        .collect()
}
#[test]
fn native_ssr_sources_align_with_original_target_stamps_and_independent_pcm_oracle() {
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-pcm.f32le");
    let blob = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-packets.bin");
    for case in manifest()["cases"].as_array().unwrap() {
        let sources = packets(&case["source_video"]);
        let mut decoder =
            NativeAacDecoder::new(&hex(case["source_asc"].as_str().unwrap())).unwrap();
        let mut alignment = SsrPcmAlignment::new(1, 2).unwrap();
        let gains = [OutputGain {
            channel: 0,
            gain: 1.0,
        }];
        let mut emitted = vec![];
        let mut position = 0u64;
        let mut previous = 0u64;
        for (i, packet) in sources.iter().enumerate() {
            let descriptor = &case["source_frames"][i];
            let off = descriptor["offset"].as_u64().unwrap() as usize;
            assert_eq!(
                packet,
                &blob[off..off + descriptor["bytes"].as_u64().unwrap() as usize]
            );
            let source = decoder.decode(packet).unwrap();
            assert_eq!(
                source.len(),
                descriptor["samples"].as_u64().unwrap() as usize
            );
            let rows = case["frames"][i]["samples"].as_u64().unwrap() as usize;
            let target = vec![0.0; rows];
            let inputs = [
                LaneInput {
                    samples: &target,
                    outputs: &gains,
                },
                LaneInput {
                    samples: &source,
                    outputs: &gains,
                },
            ];
            let checkpoint = alignment.clone();
            let rendered = alignment.submit(position, rows, &inputs).unwrap();
            alignment = checkpoint;
            assert_eq!(alignment.submit(position, rows, &inputs).unwrap(), rendered);
            if let Some(frame) = rendered {
                assert_eq!(frame.stamp, previous);
                emitted.extend(frame.samples);
            } else {
                assert_eq!(i, 0);
            }
            previous = position;
            position += rows as u64;
        }
        let last = alignment.finish().unwrap().unwrap();
        assert_eq!(last.stamp, previous);
        emitted.extend(last.samples);
        assert_eq!(emitted.len(), 6144);
        let off = case["pcm_offset"].as_u64().unwrap() as usize;
        for (i, (a, b)) in emitted
            .iter()
            .zip(gold[off..off + emitted.len() * 4].chunks_exact(4))
            .enumerate()
        {
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 2e-7,
                "{} sample {i}: {a} vs {b}",
                case["name"]
            );
        }
    }
}
#[test]
fn coupled_videos_delay_original_source_timing_and_checkpoint_exactly() {
    for case in manifest()["cases"].as_array().unwrap() {
        let encoded = packets(&case["video"]);
        let mut decoder = NativeAacDecoder::new(&hex(case["asc"].as_str().unwrap())).unwrap();
        let mut position = -100i64;
        let mut previous = None;
        let mut output = Vec::new();
        for (i, packet) in encoded.iter().enumerate() {
            let rows = case["frames"][i]["samples"].as_u64().unwrap();
            let was_delayed = decoder.delayed();
            let starts_alignment =
                !was_delayed && rows != case["source_frames"][i]["samples"].as_u64().unwrap();
            let saved = decoder.checkpoint();
            if was_delayed {
                assert!(decoder.decode_timed(&[], position, rows).is_err());
            }
            let actual = decoder.decode_timed(packet, position, rows).unwrap();
            decoder.restore(&saved).unwrap();
            assert_eq!(
                decoder.decode_timed(packet, position, rows).unwrap(),
                actual
            );
            if starts_alignment {
                assert!(actual.is_none());
                assert!(decoder.delayed());
            }
            if let Some(frame) = actual {
                let (pts, duration) = if !was_delayed {
                    (position, rows)
                } else {
                    previous.unwrap()
                };
                assert_eq!((frame.pts, frame.duration), (pts, duration));
                assert_eq!(frame.samples.len(), duration as usize);
                output.extend(frame.samples);
            }
            previous = Some((position, rows));
            position += rows as i64;
        }
        let saved = decoder.checkpoint();
        let final_frame = decoder.finish().unwrap().unwrap();
        assert_eq!((final_frame.pts, final_frame.duration), previous.unwrap());
        assert!(decoder.finish().unwrap().is_none());
        decoder.restore(&saved).unwrap();
        assert_eq!(decoder.finish().unwrap().unwrap(), final_frame);
        output.extend(final_frame.samples);
        assert_eq!(output.len(), 6144);
        decoder.reset();
        assert!(!decoder.delayed());
        let mut replay = Vec::new();
        let mut at = -100;
        for (i, packet) in encoded.iter().enumerate() {
            let rows = case["frames"][i]["samples"].as_u64().unwrap();
            if let Some(frame) = decoder.decode_timed(packet, at, rows).unwrap() {
                replay.extend(frame.samples);
            }
            at += rows as i64;
        }
        replay.extend(decoder.finish().unwrap().unwrap().samples);
        assert_eq!(replay, output);
    }
}
#[test]
fn independently_switched_ssr_coupled_videos_have_native_playback_acceptance() {
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-pcm.f32le");
    for case in acceptance_cases() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(path).unwrap();
        let mut actual = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, case["samples"].as_u64().unwrap());
        assert_eq!(
            actual.len(),
            case["samples"].as_u64().unwrap() as usize
                * 4
                * case["channels"].as_u64().unwrap() as usize
        );
        let offset = case["pcm_offset"].as_u64().unwrap() as usize;
        for (a, e) in actual
            .chunks_exact(4)
            .zip(gold[offset..offset + actual.len()].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(e.try_into().unwrap()))
                .abs()
                    < 2e-7
            );
        }
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut root).unwrap();
        assert_eq!(root, actual);
    }
}

#[cfg(feature = "player")]
#[test]
fn aligned_ssr_playback_drains_eof_and_replays_seek_ranges_with_original_timing() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            let checkpoint = decoder.checkpoint().unwrap();
            let frame = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            decoder.restore(&checkpoint).unwrap();
            let replay = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            assert_eq!(
                frame
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration)),
                replay
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration))
            );
            if let Some(frame) = frame {
                if let Some(pcm) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    out.extend(pcm.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                out.extend(pcm.data);
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        out
    }
    for case in acceptance_cases() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let data = std::fs::read(path).unwrap();
        let mut expected = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let size = case["channels"].as_u64().unwrap() as usize * 4;
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for at in [1100, 2600, 5900, 6144] {
            let landed = reader.seek_to(at);
            assert_eq!(
                play(&mut reader),
                expected[landed as usize * size..],
                "{} seek {at}",
                case["name"]
            );
        }
        for _ in 0..2 {
            for (from, to) in [(10, 150), (100, 256)] {
                let interval = Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                ));
                let mut selected = vec![];
                fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                    Cursor::new(&data),
                    &mut selected,
                    interval,
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(
                    selected,
                    expected
                        [from as usize * 24 * size..(to as usize * 24 * size).min(expected.len())]
                );
                let mut root = vec![];
                fvid::native_media::decode_mp4_aac_pcm_interval(&data, &mut root, interval)
                    .unwrap();
                assert_eq!(root, selected);
            }
        }
    }
}

#[test]
fn unfinished_ssr_video_refuses_eof_without_discarding_pending_pcm() {
    let case = &manifest()["incomplete"];
    let encoded = packets(&case["video"]);
    let mut decoder = NativeAacDecoder::new(&hex(case["asc"].as_str().unwrap())).unwrap();
    assert!(
        decoder
            .decode_timed(&encoded[0], 0, 1024)
            .unwrap()
            .is_some()
    );
    assert!(
        decoder
            .decode_timed(&encoded[1], 1024, 1024)
            .unwrap()
            .is_none()
    );
    let saved = decoder.checkpoint();
    let before = decoder.retained_payload_bytes().unwrap();
    assert_eq!(
        decoder.finish().unwrap_err().to_string(),
        case["error"].as_str().unwrap()
    );
    assert_eq!(decoder.retained_payload_bytes().unwrap(), before);
    assert_eq!(
        decoder.finish().unwrap_err().to_string(),
        case["error"].as_str().unwrap()
    );
    let continuation = packets(&manifest()["cases"][0]["video"]);
    let actual = decoder.decode_timed(&continuation[2], 2048, 1024).unwrap();
    decoder.restore(&saved).unwrap();
    assert_eq!(
        decoder.decode_timed(&continuation[2], 2048, 1024).unwrap(),
        actual
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(case["video"]["file"].as_str().unwrap());
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut vec![],
            None,
            &Default::default()
        )
        .unwrap_err()
        .to_string(),
        case["error"].as_str().unwrap()
    );
}

#[cfg(feature = "player")]
#[test]
fn delayed_aac_adapter_retains_negative_source_window_and_duration() {
    use fvid::audio::AudioDecode;
    let manifest = manifest();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "source-starts-ahead")
        .unwrap();
    let encoded = packets(&case["video"]);
    let esds = fvid::container::adts::esds_for(&hex(case["asc"].as_str().unwrap())).unwrap();
    let mut decoder = fvid::codec::aac_decoder::AacDecoder::new(&esds, 24000, 1).unwrap();
    assert!(
        decoder
            .decode_packet(&encoded[0], -100, 1024)
            .unwrap()
            .is_none()
    );
    let checkpoint = decoder.checkpoint().unwrap();
    let frame = decoder
        .decode_packet(&encoded[1], 924, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(
        (frame.source_pts, frame.source_duration, frame.packet.pts),
        (-100, 1024, 0)
    );
    decoder.restore(&checkpoint).unwrap();
    let replay = decoder
        .decode_packet(&encoded[1], 924, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(replay.packet.data, frame.packet.data);
    assert_eq!((replay.source_pts, replay.source_duration), (-100, 1024));
}

#[test]
fn aligned_ssr_fixed_clock_adts_and_matroska_exports_match_mp4() {
    let manifest = manifest();
    let folder =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for case in manifest["fixed_cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(manifest["arrival_cases"].as_array().unwrap())
    {
        let mp4 = std::fs::read(folder.join(case["video"]["file"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(folder.join(case["matroska"]["file"].as_str().unwrap())).unwrap();
        for (interval, first, last) in [
            (None, 0, 6144),
            (
                Some((
                    std::time::Duration::from_millis(10),
                    std::time::Duration::from_millis(150),
                )),
                240,
                3600,
            ),
        ] {
            let mut expected = vec![];
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&mp4),
                &mut expected,
                None,
                &Default::default(),
            )
            .unwrap();
            let size = case["channels"].as_u64().unwrap() as usize * 4;
            let mut actual = vec![];
            let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&mkv),
                &mut actual,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, last - first);
            assert_eq!(
                actual,
                expected[first as usize * size..last as usize * size],
                "{} Matroska",
                case["name"]
            );
            let mut root = vec![];
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut root, interval)
                .unwrap();
            assert_eq!(root, actual);
        }
    }
    let case = &manifest["channels_cases"][0];
    let adts = std::fs::read(folder.join(case["adts"]["file"].as_str().unwrap())).unwrap();
    let mp4 = std::fs::read(
        folder.join(
            manifest["fixed_cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["channels"] == 2)
                .unwrap()["video"]["file"]
                .as_str()
                .unwrap(),
        ),
    )
    .unwrap();
    let mut expected = vec![];
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&mp4),
        &mut expected,
        None,
        &Default::default(),
    )
    .unwrap();
    for (interval, first, last) in [
        (None, 0, 6144),
        (
            Some((
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(150),
            )),
            240,
            3600,
        ),
    ] {
        let mut actual = vec![];
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&adts),
            &mut actual,
            interval,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, last - first);
        assert_eq!(actual, expected[first as usize * 8..last as usize * 8]);
        let mut root = vec![];
        fvid::native_media::decode_aac_pcm_interval(
            &adts,
            &mut root,
            &Default::default(),
            interval,
        )
        .unwrap();
        assert_eq!(root, actual);
    }
}

#[test]
fn absent_ssr_cce_preserves_pending_pcm_and_checkpoint_replay() {
    let manifest = manifest();
    let case = &manifest["roster"];
    let encoded = packets(&case["video"]);
    let mut decoder = NativeAacDecoder::new(&hex(case["asc"].as_str().unwrap())).unwrap();
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-pcm.f32le");
    let offset = manifest["cases"][0]["pcm_offset"].as_u64().unwrap() as usize;
    let retained = case["retained_source_samples"].as_u64().unwrap() as usize;
    let mut expected = gold[offset..offset + retained * 4].to_vec();
    expected.resize(case["accepted_samples"].as_u64().unwrap() as usize * 4, 0);
    let mut pcm = Vec::new();
    for (i, packet) in encoded.iter().enumerate() {
        let saved = decoder.checkpoint();
        assert!(decoder.decode_timed(&[], i as i64 * 1024, 1024).is_err());
        let first = decoder.decode_timed(packet, i as i64 * 1024, 1024).unwrap();
        decoder.restore(&saved).unwrap();
        let again = decoder.decode_timed(packet, i as i64 * 1024, 1024).unwrap();
        assert_eq!(first, again);
        if let Some(frame) = again {
            assert_eq!(frame.pts, (pcm.len() / 4) as i64);
            pcm.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
        }
    }
    let saved = decoder.checkpoint();
    let tail = decoder.finish().unwrap();
    decoder.restore(&saved).unwrap();
    assert_eq!(decoder.finish().unwrap(), tail);
    if let Some(frame) = tail {
        pcm.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
    }
    assert!(decoder.finish().unwrap().is_none());
    assert_eq!(pcm, expected);
}

#[test]
fn late_ssr_cce_sources_enter_without_backdating_pcm_or_losing_old_history() {
    let manifest = manifest();
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-alignment-pcm.f32le");
    for case in manifest["arrival_cases"].as_array().unwrap() {
        let encoded = packets(&case["video"]);
        let mut decoder = NativeAacDecoder::new(&hex(case["asc"].as_str().unwrap())).unwrap();
        let mut output = Vec::new();
        for (i, packet) in encoded.iter().enumerate() {
            let checkpoint = decoder.checkpoint();
            let frame = decoder
                .decode_timed(packet, (i * 1024) as i64, 1024)
                .unwrap();
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(
                decoder
                    .decode_timed(packet, (i * 1024) as i64, 1024)
                    .unwrap(),
                frame
            );
            if let Some(frame) = frame {
                let origin = if i < 2 { i } else { i - 1 };
                assert_eq!((frame.pts, frame.duration), ((origin * 1024) as i64, 1024));
                output.extend(frame.samples);
            } else {
                assert_eq!(i, 1);
            }
        }
        let final_frame = decoder.finish().unwrap().unwrap();
        assert_eq!(final_frame.pts, 5120);
        output.extend(final_frame.samples);
        assert_eq!(output.len(), 6144);
        let off = case["pcm_offset"].as_u64().unwrap() as usize;
        for (i, (&actual, bytes)) in output
            .iter()
            .zip(gold[off..off + output.len() * 4].chunks_exact(4))
            .enumerate()
        {
            let expected = f32::from_le_bytes(bytes.try_into().unwrap());
            assert!(
                (actual - expected).abs() < 2e-7,
                "{} sample {i}: {actual} != {expected}",
                case["name"]
            );
        }
    }
}
