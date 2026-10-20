#![cfg(feature = "player")]
use fvid::{audio::AudioStream, container::webm::Limits, playback_webm_audio::WebmAudioReader};
use std::io::Cursor;

fn decode(bytes: &[u8], seek: Option<i64>) -> (Vec<f32>, Vec<u64>, u16) {
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    assert_eq!(stream.sample_rate(), 48000);
    let channels = stream.channels();
    let mut decoder =
        fvid::codec::make_audio_decoder(stream.codec(), stream.extra_data(), 48000, channels, 0)
            .unwrap();
    if let Some(target) = seek {
        assert_eq!(stream.seek_to(target), target);
        decoder.reset();
    }
    let mut output = Vec::new();
    let mut stamps = Vec::new();
    let mut end = 0u128;
    while let Some(packet) = stream.next_packet().unwrap() {
        let pcm = decoder
            .decode_encoded(
                &packet.data,
                packet.pts.max(0) as u64,
                packet.duration as u64,
            )
            .unwrap()
            .unwrap();
        if let Some(pcm) = stream.present_decoded(pcm, packet.pts).unwrap() {
            stamps.push(pcm.pts);
            end = u128::from(pcm.pts)
                + (pcm.data.len() / (usize::from(channels) * 4)) as u128 * 1_000_000_000 / 48000;
            output.extend(
                pcm.data
                    .chunks_exact(4)
                    .map(|s| f32::from_le_bytes(s.try_into().unwrap())),
            );
        }
    }
    assert!(stamps.windows(2).all(|s| s[0] < s[1]));
    assert!(stream.duration().unwrap().as_nanos().abs_diff(end) <= 1);
    (output, stamps, channels)
}

fn check(bytes: &[u8], reference: &[u8], channels: u16, mode: u8, origin: u64) {
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    while let Some(packet) = stream.next_packet().unwrap() {
        let config = packet.data[0] >> 3;
        let found = if config < 12 {
            0
        } else if config < 16 {
            1
        } else {
            2
        };
        assert_eq!(found, mode, "fixture does not exercise its named Opus mode");
    }
    let (pcm, stamps, found) = decode(bytes, None);
    assert_eq!(found, channels);
    assert_eq!(pcm.len(), 48000 * usize::from(channels));
    assert_eq!(stamps[0], origin);
    assert_eq!(reference.len(), pcm.len() * 4);
    let error = pcm
        .iter()
        .zip(reference.chunks_exact(4))
        .map(|(&a, b)| (a - f32::from_le_bytes(b.try_into().unwrap())).abs())
        .fold(0.0f32, f32::max);
    assert!(
        error < 0.00002,
        "Opus PCM differs from libopus: max_error={error}"
    );
    let (again, _, _) = decode(bytes, None);
    assert_eq!(again, pcm);
    for target in [origin as i64, 500_000_000, 999_000_000] {
        let (tail, stamps, _) = decode(bytes, Some(target));
        assert!(!tail.is_empty());
        assert!(stamps[0] >= target as u64 && stamps[0] - (target as u64) < 21_000);
        // Matroska timestamps are quantized to milliseconds. Seeking uses those
        // stamps, so choose the suffix by the exact emitted sample count.
        assert_eq!(tail, pcm[pcm.len() - tail.len()..]);
    }
}

#[test]
fn mono_celt_matches_libopus_and_trims_priming_and_tail() {
    check(
        include_bytes!("fixtures/playback-errors/opus-mono.webm"),
        include_bytes!("fixtures/playback-errors/opus-mono.f32"),
        1,
        2,
        0,
    );
}
#[test]
fn stereo_celt_preserves_both_channels() {
    check(
        include_bytes!("fixtures/playback-errors/opus-stereo.webm"),
        include_bytes!("fixtures/playback-errors/opus-stereo.f32"),
        2,
        2,
        0,
    );
}
#[test]
fn silk_speech_mode_matches_libopus() {
    check(
        include_bytes!("fixtures/playback-errors/opus-silk.webm"),
        include_bytes!("fixtures/playback-errors/opus-silk.f32"),
        1,
        0,
        0,
    );
}
#[test]
fn hybrid_silk_celt_matches_libopus() {
    check(
        include_bytes!("fixtures/playback-errors/opus-hybrid.webm"),
        include_bytes!("fixtures/playback-errors/opus-hybrid.f32"),
        1,
        1,
        0,
    );
}

#[test]
fn surround_51_matches_libopus_in_wave_channel_order() {
    check(
        include_bytes!("fixtures/playback-errors/opus-surround.webm"),
        include_bytes!("fixtures/playback-errors/opus-surround.f32"),
        6,
        2,
        0,
    );
}

#[test]
fn invalid_headers_and_packets_are_reported_and_reset_restores_pcm() {
    use fvid::audio::AudioDecode;
    let bytes = include_bytes!("fixtures/playback-errors/opus-mono.webm");
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    let mut head = stream.extra_data().to_vec();
    assert!(fvid::codec::opus_decoder::OpusDecoder::new(&head[..18], 48000, 1).is_err());
    assert!(fvid::codec::opus_decoder::OpusDecoder::new(&head, 44100, 1).is_err());
    assert!(fvid::codec::opus_decoder::OpusDecoder::new(&head, 48000, 2).is_err());
    let mut decoder = fvid::codec::opus_decoder::OpusDecoder::new(&head, 48000, 1).unwrap();
    let packet = stream.next_packet().unwrap().unwrap();
    let first = decoder
        .decode_encoded(&packet.data, 0, 0)
        .unwrap()
        .unwrap()
        .data;
    assert!(decoder.decode_encoded(&[], 0, 0).is_err());
    assert!(decoder.decode_encoded(&[3], 0, 0).is_err());
    decoder.reset();
    assert_eq!(
        decoder
            .decode_encoded(&packet.data, 0, 0)
            .unwrap()
            .unwrap()
            .data,
        first
    );
    head[16..18].copy_from_slice(&1536i16.to_le_bytes());
    let mut louder = fvid::codec::opus_decoder::OpusDecoder::new(&head, 48000, 1).unwrap();
    let scaled = louder
        .decode_encoded(&packet.data, 0, 0)
        .unwrap()
        .unwrap()
        .data;
    for (a, b) in first.chunks_exact(4).zip(scaled.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((b - a * 10f32.powf(6.0 / 20.0)).abs() < 0.000001);
    }
}

#[test]
fn positive_first_packet_still_discards_encoder_priming() {
    let bytes = include_bytes!("fixtures/playback-errors/opus-positive-first.webm");
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    let first = stream.read_packet().unwrap().unwrap();
    assert!(
        first.pts_ns > 0,
        "fixture must reproduce the positive first packet"
    );
    check(
        bytes,
        include_bytes!("fixtures/playback-errors/opus-positive-first.f32"),
        1,
        2,
        1_500_000,
    );
}

#[test]
fn hevc_companion_multistream_timing_and_padding_preserve_playback_and_seek_samples() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for file in [
        "hevc-opus-surround-companion.mkv",
        "hevc-opus-surround-no-duration.mkv",
        "hevc-opus-surround-padded-stream.mkv",
        "hevc-opus-discrete-companion.mkv",
        "hevc-opus-remapped-surround.mkv",
    ] {
        let bytes = std::fs::read(root.join(file)).unwrap();
        let mut expected = Vec::new();
        let stats = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(&bytes),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let (pcm, _, channels) = decode(&bytes, None);
        assert_eq!(channels, 6);
        assert_eq!(stats.channels, channels);
        let actual: Vec<u8> = pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        assert_eq!(actual, expected, "{file}: streaming vs owned presentation");
        let (tail, stamps, _) = decode(&bytes, Some(37_123_000));
        assert!(!tail.is_empty());
        assert!(stamps[0] >= 37_123_000 && stamps[0] - 37_123_000 < 21_000);
        assert_eq!(
            tail,
            pcm[pcm.len() - tail.len()..],
            "{file}: exact seek suffix"
        );
    }
}

#[test]
fn wide_discrete_opus_playback_and_seek_preserve_all_declared_channels() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let baseline = decode(
        &std::fs::read(root.join("hevc-opus-discrete-companion.mkv")).unwrap(),
        None,
    )
    .0;
    for channels in [9usize, 255] {
        let bytes = std::fs::read(root.join(format!("hevc-opus-discrete-{channels}.mkv"))).unwrap();
        let (pcm, _, found) = decode(&bytes, None);
        assert_eq!(usize::from(found), channels);
        assert_eq!(pcm.len() / channels, baseline.len() / 6);
        for (a, b) in pcm.chunks_exact(channels).zip(baseline.chunks_exact(6)) {
            for c in 0..channels {
                assert_eq!(a[c], b[c % 6], "channels={channels} channel={c}");
            }
        }
        let (tail, stamps, found) = decode(&bytes, Some(37_123_000));
        assert_eq!(usize::from(found), channels);
        assert!(!tail.is_empty());
        assert!(stamps[0] >= 37_123_000 && stamps[0] - 37_123_000 < 21_000);
        assert_eq!(tail, pcm[pcm.len() - tail.len()..]);
    }
}

#[test]
fn ogg_opus_preskip_end_trim_seek_and_rewind_match_native_video_audio() {
    use fvid::audio::{AudioDecode, AudioStream};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let video = std::fs::read(root.join("hevc-opus-surround-companion.mkv")).unwrap();
    let mut expected = Vec::new();
    fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(&video),
        &mut expected,
        None,
        &Default::default(),
    )
    .unwrap();
    expected.truncate(expected.len() - 137 * 6 * 4);
    for name in [
        "opus-surround-presentation.opus",
        "opus-surround-padded-presentation.opus",
    ] {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut stream =
            fvid::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default())
                .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        let collect = |stream: &mut fvid::playback_ogg_opus::OggOpusAudioReader,
                       decoder: &mut dyn AudioDecode| {
            let mut pcm = Vec::new();
            let mut stamps = Vec::new();
            while let Some(packet) = stream.next_packet().unwrap() {
                let decoded = decoder
                    .decode_encoded(
                        &packet.data,
                        packet.pts.max(0) as u64,
                        packet.duration as u64,
                    )
                    .unwrap()
                    .unwrap();
                if let Some(presented) = stream.present_decoded(decoded, packet.pts).unwrap() {
                    assert_eq!(presented.timebase_den, 48000);
                    stamps.push(presented.pts);
                    pcm.extend(presented.data);
                }
            }
            assert!(stamps.windows(2).all(|w| w[0] < w[1]));
            (pcm, stamps)
        };
        assert_eq!(stream.timescale(), 48000);
        assert_eq!(stream.channels(), 6);
        assert_eq!(
            stream.duration().unwrap().as_nanos(),
            (expected.len() / 24) as u128 * 1_000_000_000 / 48000
        );
        let (full, stamps) = collect(&mut stream, decoder.as_mut());
        assert_eq!(stamps[0], 0);
        assert_eq!(full, expected, "{name}");
        assert_eq!(stream.seek_to(1782), 1782);
        decoder.reset();
        let (tail, stamps) = collect(&mut stream, decoder.as_mut());
        assert_eq!(stamps[0], 1782);
        assert_eq!(tail, expected[1782 * 24..]);
        stream.rewind();
        decoder.reset();
        assert_eq!(collect(&mut stream, decoder.as_mut()).0, expected);
    }
}

#[test]
fn chained_ogg_opus_resets_history_and_preserves_exact_seek_across_segment_windows() {
    use fvid::audio::{AudioDecode, AudioStep, AudioStream};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let video = std::fs::read(root.join("hevc-opus-surround-companion.mkv")).unwrap();
    let mut original = Vec::new();
    fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(&video),
        &mut original,
        None,
        &Default::default(),
    )
    .unwrap();
    let mut expected = original[..5311 * 24].to_vec();
    expected.extend_from_slice(&original[336 * 24..5237 * 24]);
    let bytes = std::fs::read(root.join("opus-surround-chained.opus")).unwrap();
    let parsed = fvid::container::ogg::Ogg::parse(&bytes, &Default::default()).unwrap();
    assert_eq!(parsed.chains().len(), 2);
    assert!(parsed.chains()[0].last_page < parsed.chains()[1].first_page);
    let mut stream =
        fvid::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default()).unwrap();
    let mut decoder = stream.make_decoder().unwrap();
    let collect = |stream: &mut fvid::playback_ogg_opus::OggOpusAudioReader,
                   decoder: &mut dyn AudioDecode| {
        let mut pcm = Vec::new();
        let mut stamps = Vec::new();
        let mut resets = 0;
        while let Some(step) = stream.next_step().unwrap() {
            match step {
                AudioStep::ResetDecoder => {
                    decoder.reset();
                    resets += 1;
                }
                AudioStep::Encoded(packet) => {
                    let decoded = decoder
                        .decode_encoded(
                            &packet.data,
                            packet.pts.max(0) as u64,
                            packet.duration as u64,
                        )
                        .unwrap()
                        .unwrap();
                    if let Some(p) = stream.present_decoded(decoded, packet.pts).unwrap() {
                        stamps.push(p.pts);
                        pcm.extend(p.data);
                    }
                }
                _ => panic!("Opus must stay in the native codec path"),
            }
        }
        assert!(stamps.windows(2).all(|w| w[0] < w[1]));
        (pcm, stamps, resets)
    };
    assert_eq!(
        stream.duration().unwrap().as_nanos(),
        10212u128 * 1_000_000_000 / 48000
    );
    let (pcm, stamps, resets) = collect(&mut stream, decoder.as_mut());
    assert_eq!(pcm, expected);
    assert_eq!(stamps[0], 0);
    assert_eq!(resets, 1);
    for target in [0usize, 1782, 5310, 5311, 5312, 7000, 10211, 10212] {
        assert_eq!(stream.seek_to(target as i64), target as i64);
        decoder.reset();
        let (pcm, stamps, _) = collect(&mut stream, decoder.as_mut());
        assert_eq!(pcm, expected[target * 24..], "seek={target}");
        if !stamps.is_empty() {
            assert_eq!(stamps[0], target as u64);
        }
    }
    stream.rewind();
    decoder.reset();
    assert_eq!(collect(&mut stream, decoder.as_mut()).0, expected);
}

#[test]
fn parallel_ogg_opus_tracks_preserve_selection_seek_and_chained_link_identity() {
    use fvid::audio::{AudioStep, AudioStream};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let video = std::fs::read(root.join("hevc-opus-surround-companion.mkv")).unwrap();
    let mut original = Vec::new();
    fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(&video),
        &mut original,
        None,
        &Default::default(),
    )
    .unwrap();
    for chained in [false, true] {
        let name = if chained {
            "opus-surround-parallel-chained.opus"
        } else {
            "opus-surround-parallel.opus"
        };
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert!(
            fvid::playback_ogg_opus::OggOpusAudioReader::open_track(
                &bytes[..],
                Default::default(),
                2
            )
            .is_err()
        );
        for track in 0..2 {
            let mut expected = if track == 0 {
                original[..5311 * 24].to_vec()
            } else {
                original[336 * 24..5311 * 24].to_vec()
            };
            let boundary = expected.len() / 24;
            if chained {
                expected.extend_from_slice(if track == 0 {
                    &original[336 * 24..5237 * 24]
                } else {
                    &original[..5311 * 24]
                });
            }
            let mut stream = fvid::playback_ogg_opus::OggOpusAudioReader::open_track(
                &bytes[..],
                Default::default(),
                track,
            )
            .unwrap();
            let tracks = stream.audio_tracks();
            assert_eq!(tracks.len(), 2);
            assert_eq!(tracks[0].name, "Left program");
            assert_eq!(tracks[1].name, "Right program");
            assert_eq!(tracks[0].language, "eng");
            assert_eq!(tracks[1].language, "rus");
            assert_eq!(
                stream.duration().unwrap().as_nanos(),
                (expected.len() / 24) as u128 * 1_000_000_000 / 48000
            );
            let mut decoder = stream.make_decoder().unwrap();
            let total = expected.len() / 24;
            for target in [
                0usize,
                1700,
                boundary.saturating_sub(1),
                boundary.min(total),
                total.saturating_sub(1),
                total,
            ] {
                assert_eq!(stream.seek_to(target as i64), target as i64);
                decoder.reset();
                let mut pcm = Vec::new();
                let mut stamps = Vec::new();
                let mut resets = 0;
                while let Some(step) = stream.next_step().unwrap() {
                    match step {
                        AudioStep::ResetDecoder => {
                            decoder.reset();
                            resets += 1;
                        }
                        AudioStep::Encoded(packet) => {
                            let decoded = decoder
                                .decode_encoded(
                                    &packet.data,
                                    packet.pts.max(0) as u64,
                                    packet.duration as u64,
                                )
                                .unwrap()
                                .unwrap();
                            if let Some(p) = stream.present_decoded(decoded, packet.pts).unwrap() {
                                stamps.push(p.pts);
                                pcm.extend(p.data);
                            }
                        }
                        _ => panic!("Opus must use the native codec"),
                    }
                }
                assert_eq!(
                    pcm,
                    expected[target * 24..],
                    "{name} track={track} seek={target}"
                );
                assert!(stamps.windows(2).all(|w| w[0] < w[1]));
                if !stamps.is_empty() {
                    assert_eq!(stamps[0], target as u64);
                }
                assert_eq!(resets, usize::from(chained && target < boundary));
            }
        }
    }
}

#[test]
fn ogg_opus_mapping_and_gain_changes_rebuild_decoder_for_boundaries_seek_and_rewind() {
    use fvid::audio::{AudioStep, AudioStream};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let mut expected = Vec::new();
    for (name, trim) in [
        ("hevc-opus-surround-companion.mkv", 137),
        ("hevc-opus-reconfigured-companion.mkv", 211),
    ] {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut pcm = Vec::new();
        fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        pcm.truncate(pcm.len() - trim * 24);
        expected.extend(pcm);
    }
    assert_eq!(expected.len() / 24, 10212);
    let bytes = std::fs::read(root.join("opus-surround-reconfigured.opus")).unwrap();
    let mut stream =
        fvid::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default()).unwrap();
    let mut decoder = stream.make_decoder().unwrap();
    for target in [0usize, 5310, 5311, 5312, 7000, 10211, 0] {
        assert_eq!(stream.seek_to(target as i64), target as i64);
        stream.reset_decoder(&mut decoder).unwrap();
        let mut pcm = Vec::new();
        let mut resets = 0;
        while let Some(step) = stream.next_step().unwrap() {
            match step {
                AudioStep::ResetDecoder => {
                    stream.reset_decoder(&mut decoder).unwrap();
                    resets += 1;
                }
                AudioStep::Encoded(packet) => {
                    let decoded = decoder
                        .decode_encoded(
                            &packet.data,
                            packet.pts.max(0) as u64,
                            packet.duration as u64,
                        )
                        .unwrap()
                        .unwrap();
                    if let Some(p) = stream.present_decoded(decoded, packet.pts).unwrap() {
                        pcm.extend(p.data);
                    }
                }
                _ => panic!("Opus must stay native"),
            }
        }
        assert_eq!(pcm, expected[target * 24..], "seek={target}");
        assert_eq!(resets, usize::from(target < 5311));
    }
    stream.rewind();
    stream.reset_decoder(&mut decoder).unwrap();
    assert_eq!(
        fvid_opus::OpusHead::parse(stream.extra_data())
            .unwrap()
            .output_gain_q8,
        0
    );
}

#[test]
fn changing_ogg_opus_channel_counts_preserve_speakers_in_one_output_layout() {
    use fvid::audio::{AudioStep, AudioStream};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let decode_video = |name: &str| {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut pcm = Vec::new();
        fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        pcm
    };
    let six = decode_video("hevc-opus-surround-companion.mkv");
    let quad = decode_video("hevc-opus-quad-companion.mkv");
    let extend_quad = |bytes: &[u8]| {
        let mut result = Vec::new();
        for row in bytes.chunks_exact(16) {
            result.extend_from_slice(&row[..8]);
            result.extend([0; 8]);
            result.extend_from_slice(&row[8..]);
        }
        result
    };
    for reverse in [false, true] {
        let name = if reverse {
            "opus-quad-to-surround.opus"
        } else {
            "opus-surround-to-quad.opus"
        };
        let mut expected = if reverse {
            extend_quad(&quad[..5311 * 16])
        } else {
            six[..5311 * 24].to_vec()
        };
        expected.extend(if reverse {
            six[336 * 24..5237 * 24].to_vec()
        } else {
            extend_quad(&quad[336 * 16..5237 * 16])
        });
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut stream =
            fvid::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default())
                .unwrap();
        assert_eq!(stream.channels(), 6);
        assert_eq!(stream.audio_tracks()[0].channels, 6);
        assert_eq!(stream.output_channel_mask(), Some(0x3f));
        assert_eq!(
            fvid_opus::OpusHead::parse(stream.extra_data())
                .unwrap()
                .channel_count,
            if reverse { 4 } else { 6 }
        );
        let mut decoder = stream.make_decoder().unwrap();
        for target in [0usize, 1782, 5310, 5311, 5312, 7000, 10211, 10212, 0] {
            assert_eq!(stream.seek_to(target as i64), target as i64);
            stream.reset_decoder(&mut decoder).unwrap();
            let mut actual = Vec::new();
            let mut stamps = Vec::new();
            while let Some(step) = stream.next_step().unwrap() {
                match step {
                    AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                    AudioStep::Encoded(packet) => {
                        let decoded = decoder
                            .decode_encoded(
                                &packet.data,
                                packet.pts.max(0) as u64,
                                packet.duration as u64,
                            )
                            .unwrap()
                            .unwrap();
                        if let Some(p) = stream.present_decoded(decoded, packet.pts).unwrap() {
                            assert_eq!(p.data.len() % 24, 0);
                            stamps.push(p.pts);
                            actual.extend(p.data);
                        }
                    }
                    _ => panic!("Opus must stay native"),
                }
            }
            assert_eq!(actual, expected[target * 24..], "{name} seek={target}");
            assert!(stamps.windows(2).all(|w| w[0] < w[1]));
            if !stamps.is_empty() {
                assert_eq!(stamps[0], target as u64);
            }
        }
        stream.rewind();
        stream.reset_decoder(&mut decoder).unwrap();
        assert_eq!(
            fvid_opus::OpusHead::parse(stream.extra_data())
                .unwrap()
                .channel_count,
            if reverse { 4 } else { 6 }
        );
    }
}

#[test]
fn mono_and_quad_ogg_segments_preserve_named_speakers_across_mapping_families() {
    use fvid::audio::{AudioStep, AudioStream};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let decode_video = |name: &str| {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut pcm = Vec::new();
        fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        pcm
    };
    let mono = decode_video("hevc-opus-mono-companion.mkv");
    let quad = decode_video("hevc-opus-quad-companion.mkv");
    let mono_out = |pcm: &[u8]| {
        let mut out = Vec::new();
        for v in pcm.chunks_exact(4) {
            out.extend([0; 8]);
            out.extend_from_slice(v);
            out.extend([0; 8]);
        }
        out
    };
    let quad_out = |pcm: &[u8]| {
        let mut out = Vec::new();
        for v in pcm.chunks_exact(16) {
            out.extend_from_slice(&v[..8]);
            out.extend([0; 4]);
            out.extend_from_slice(&v[8..]);
        }
        out
    };
    for reverse in [false, true] {
        let file = if reverse {
            "opus-quad-to-mono.opus"
        } else {
            "opus-mono-to-quad.opus"
        };
        let mut expected = if reverse {
            quad_out(&quad[..5311 * 16])
        } else {
            mono_out(&mono[..5311 * 4])
        };
        expected.extend(if reverse {
            mono_out(&mono[336 * 4..5237 * 4])
        } else {
            quad_out(&quad[336 * 16..5237 * 16])
        });
        let bytes = std::fs::read(root.join(file)).unwrap();
        let mut stream =
            fvid::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default())
                .unwrap();
        assert_eq!(stream.channels(), 5);
        assert_eq!(stream.output_channel_mask(), Some(0x37));
        let mut decoder = stream.make_decoder().unwrap();
        for target in [0usize, 5310, 5311, 7000, 0] {
            stream.seek_to(target as i64);
            stream.reset_decoder(&mut decoder).unwrap();
            let mut actual = Vec::new();
            while let Some(step) = stream.next_step().unwrap() {
                match step {
                    AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                    AudioStep::Encoded(p) => {
                        let decoded = decoder
                            .decode_encoded(&p.data, p.pts.max(0) as u64, p.duration as u64)
                            .unwrap()
                            .unwrap();
                        if let Some(p) = stream.present_decoded(decoded, p.pts).unwrap() {
                            assert_eq!(p.data.len() % 20, 0);
                            actual.extend(p.data);
                        }
                    }
                    _ => panic!("Opus must stay native"),
                }
            }
            assert_eq!(actual, expected[target * 20..], "{file} seek={target}");
        }
    }
}
