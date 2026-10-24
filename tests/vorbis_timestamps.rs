#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use fvid::playback_ogg_audio::OggAudioReader;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn vorbis_granules_mixed_modes_chains_and_seek_present_exact_samples() {
    for (name, end) in [
        ("vorbis-timestamps-zero.ogg", 411),
        ("vorbis-timestamps-initial-trim.ogg", 394),
        ("vorbis-timestamps-positive-origin.ogg", 434),
        ("vorbis-timestamps-chained.ogg", 805),
        ("vorbis-timestamps-reconfigured.ogg", 805),
        ("vorbis-timestamps-changing-blocks.ogg", 1189),
    ] {
        let bytes = fixture(name);
        let mut stream = OggAudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(
            stream.duration().unwrap().as_secs_f64(),
            end as f64 / 8000.,
            "{name}"
        );
        for target in [0, 19, 80, 390, 411.min(end), 500.min(end), end, end + 1, 0] {
            let landed = stream.seek_to(target);
            assert_eq!(landed, target.min(end));
            let mut decoder = stream.make_decoder().unwrap();
            let mut count = 0;
            let mut previous = None;
            while let Some(step) = stream.next_step().unwrap() {
                match step {
                    AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
                    AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                    AudioStep::Encoded(packet) => {
                        if let Some(decoded) = decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                        {
                            if let Some(pcm) = stream
                                .present_decoded(decoded.packet, decoded.source_pts)
                                .unwrap()
                            {
                                let frames = pcm.data.len() / 8;
                                assert!(frames > 0);
                                assert!(pcm.pts >= target.min(end) as u64);
                                if let Some(last) = previous {
                                    assert!(pcm.pts >= last, "{name}");
                                }
                                previous = Some(pcm.pts + frames as u64);
                                count += frames;
                            }
                        }
                    }
                    _ => panic!("unexpected Vorbis step"),
                }
            }
            // Positive initial position is a presentation gap, not invented PCM.
            let start = if name.contains("positive-origin") {
                23
            } else {
                0
            };
            assert_eq!(
                count,
                (end - target.max(start).min(end)) as usize,
                "{name} target={target}"
            );
        }
    }
}

#[test]
fn webm_vorbis_clock_and_seek_use_container_nanoseconds() {
    use fvid::playback_webm_audio::WebmAudioReader;
    for target in [0, 12_375_000, 46_125_000, 56_000_000] {
        let bytes = fixture("hevc-vorbis-timestamps-companion.mkv");
        let mut stream =
            WebmAudioReader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
        assert_eq!(stream.seek_to(target), target);
        let mut decoder = stream.make_decoder().unwrap();
        let mut count = 0;
        let mut last = None;
        while let Some(packet) = stream.next_packet().unwrap() {
            if let Some(decoded) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(pcm) = stream
                    .present_decoded(decoded.packet, decoded.source_pts)
                    .unwrap()
                {
                    assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 1_000_000_000));
                    assert!(pcm.pts >= target as u64);
                    if let Some(end) = last {
                        assert_eq!(pcm.pts, end);
                    }
                    let frames = pcm.data.len() / 8;
                    count += frames;
                    last = Some(pcm.pts + frames as u64 * 125000);
                }
            }
        }
        assert_eq!(
            count,
            448 - (target as usize * 8000).div_ceil(1_000_000_000)
        );
    }
}

#[test]
fn webm_vorbis_signed_padding_and_codec_delay_trim_exact_pcm_windows() {
    use fvid::playback_webm_audio::WebmAudioReader;
    for (name, total, after_seek) in [
        ("hevc-vorbis-fractional-tail-padding.mkv", 447, 348),
        ("hevc-vorbis-tail-padding.mkv", 411, 312),
        ("hevc-vorbis-head-padding.mkv", 431, 349),
        ("hevc-vorbis-codec-delay.mkv", 431, 332),
    ] {
        for target in [0, 12_375_000] {
            let mut stream =
                WebmAudioReader::open(std::io::Cursor::new(fixture(name)), Default::default())
                    .unwrap();
            stream.seek_to(target);
            let mut decoder = stream.make_decoder().unwrap();
            let mut count = 0;
            while let Some(packet) = stream.next_packet().unwrap() {
                if let Some(decoded) = decoder
                    .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                    .unwrap()
                {
                    if let Some(pcm) = stream
                        .present_decoded(decoded.packet, decoded.source_pts)
                        .unwrap()
                    {
                        assert_eq!(pcm.timebase_den, 1_000_000_000);
                        assert!(pcm.pts >= target as u64);
                        count += pcm.data.len() / 8;
                    }
                }
            }
            assert_eq!(
                count,
                if target == 0 { total } else { after_seek },
                "{name} target={target}"
            );
        }
    }
}

#[test]
fn webm_vorbis_endpoint_survives_rewind_and_partial_preroll() {
    use fvid::playback_webm_audio::WebmAudioReader;
    let mut stream = WebmAudioReader::open(
        std::io::Cursor::new(fixture("hevc-vorbis-endpoint-rewind.mkv")),
        Default::default(),
    )
    .unwrap();
    while stream.next_packet().unwrap().is_some() {}
    let endpoint = std::time::Duration::from_nanos(51_375_000);
    assert_eq!(stream.duration(), Some(endpoint));
    for target in [0, 12_375_000, 46_125_000, 0] {
        stream.seek_to(target);
        assert_eq!(stream.duration(), Some(endpoint), "seek {target}");
        while stream.next_packet().unwrap().is_some() {
            assert_eq!(stream.duration(), Some(endpoint), "preroll {target}");
        }
    }
}

#[test]
fn webm_fractional_vorbis_tail_endpoint_matches_presented_sample_count() {
    use fvid::playback_webm_audio::WebmAudioReader;
    let mut stream = WebmAudioReader::open(
        std::io::Cursor::new(fixture("hevc-vorbis-fractional-tail-padding.mkv")),
        Default::default(),
    )
    .unwrap();
    while stream.next_packet().unwrap().is_some() {}
    assert_eq!(
        stream.duration(),
        Some(std::time::Duration::from_nanos(447 * 125_000))
    );
}

#[test]
fn vorbis_discrete_channels_play_and_seek_in_ogg_and_matroska() {
    use fvid::playback_webm_audio::WebmAudioReader;
    for channels in [1, 9, 33, 255] {
        for container in ["ogg", "mkv"] {
            let (mut stream, total): (Box<dyn AudioStream>, usize) = if container == "ogg" {
                let bytes = fixture(&format!("vorbis-channels-{channels}.ogg"));
                (
                    Box::new(OggAudioReader::open(bytes.as_slice(), Default::default()).unwrap()),
                    411,
                )
            } else {
                let bytes = fixture(&format!("hevc-vorbis-channels-{channels}.mkv"));
                (
                    Box::new(
                        WebmAudioReader::open(std::io::Cursor::new(bytes), Default::default())
                            .unwrap(),
                    ),
                    448,
                )
            };
            assert_eq!(stream.channels(), channels);
            for sample in [0, 19, 390, total, 0] {
                let target = sample as i64 * i64::from(stream.timescale()) / 8000;
                assert_eq!(stream.seek_to(target), target);
                let mut decoder = stream.make_decoder().unwrap();
                let mut frames = 0;
                while let Some(step) = stream.next_step().unwrap() {
                    match step {
                        AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
                        AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                        AudioStep::Encoded(packet) => {
                            if let Some(decoded) = decoder
                                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                                .unwrap()
                            {
                                if let Some(pcm) = stream
                                    .present_decoded(decoded.packet, decoded.source_pts)
                                    .unwrap()
                                {
                                    assert!(pcm.pts >= target as u64);
                                    assert!(pcm.data.iter().all(|&b| b == 0));
                                    frames += pcm.data.len() / (usize::from(channels) * 4);
                                }
                            }
                        }
                        _ => panic!("unexpected Vorbis step"),
                    }
                }
                assert_eq!(
                    frames,
                    total - sample,
                    "{container} ch={channels} sample={sample}"
                );
            }
        }
    }
}

#[test]
fn vorbis_channel_changes_reset_source_decoders_without_changing_player_output() {
    for (first, second, channels) in [
        (1, 2, 3),
        (2, 1, 3),
        (4, 6, 6),
        (7, 8, 9),
        (9, 33, 33),
        (33, 9, 33),
        (2, 255, 255),
    ] {
        let bytes = fixture(&format!("vorbis-channel-chain-{first}-{second}.ogg"));
        let mut stream = OggAudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(stream.channels(), channels);
        for target in [0, 19, 410, 411, 430, 821, 822, 0] {
            stream.seek_to(target);
            let mut decoder = stream.make_decoder().unwrap();
            let mut frames = 0;
            while let Some(step) = stream.next_step().unwrap() {
                match step {
                    AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
                    AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                    AudioStep::Encoded(packet) => {
                        if let Some(decoded) = decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                        {
                            if let Some(pcm) = stream
                                .present_decoded(decoded.packet, decoded.source_pts)
                                .unwrap()
                            {
                                assert_eq!(stream.channels(), channels);
                                assert!(pcm.pts >= target as u64);
                                frames += pcm.data.len() / (channels as usize * 4);
                            }
                        }
                    }
                    _ => panic!("unexpected Vorbis step"),
                }
            }
            assert_eq!(frames, (822 - target) as usize);
        }
    }
}

#[test]
fn vorbis_linked_rate_changes_present_exact_pcm_on_one_player_clock() {
    let mut names: Vec<String> = [(8000, 12000), (12000, 8000), (44100, 48000), (48000, 44100)]
        .into_iter()
        .map(|(a, b)| format!("vorbis-rate-chain-{a}-{b}-offset-0.ogg"))
        .collect();
    names.push("vorbis-rate-chain-fractional-three.ogg".into());
    names.extend(
        [(44100, 32000), (32000, 44100)]
            .into_iter()
            .map(|(a, b)| format!("vorbis-rate-tone-chain-{a}-{b}.ogg")),
    );
    names.push("vorbis-rate-channel-chain.ogg".into());
    for name in names {
        let bytes = fixture(&name);
        let mut baseline =
            fvid_media::owned_ogg_vorbis::Program::open(bytes.as_slice(), Default::default())
                .unwrap();
        let mut expected = Vec::new();
        baseline.decode_pcm(&mut expected).unwrap();
        let width = usize::from(baseline.channels()) * 4;
        let end = expected.len() / width;
        let boundary = if name.contains("tone") { 88256 } else { 411 };
        let mut stream = OggAudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        let rate = stream.sample_rate();
        assert_eq!(stream.timescale(), rate);
        for target in [0, 19, boundary - 1, boundary, boundary + 1, end - 1, end, 0] {
            stream.seek_to(target as i64);
            let mut decoder = stream.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut cursor = target as u64;
            while let Some(step) = stream.next_step().unwrap() {
                match step {
                    AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
                    AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                    AudioStep::Encoded(packet) => {
                        if let Some(decoded) = decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                        {
                            if let Some(pcm) = stream
                                .present_decoded(decoded.packet, decoded.source_pts)
                                .unwrap()
                            {
                                assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, rate));
                                assert_eq!(pcm.pts, cursor, "{name} target {target}");
                                cursor += pcm.data.len() as u64 / width as u64;
                                actual.extend(pcm.data);
                            }
                        }
                    }
                    _ => panic!("unexpected Vorbis step"),
                }
            }
            assert_eq!(cursor, end as u64);
            assert_eq!(actual, expected[target * width..], "{name} target {target}");
        }
    }
}

#[test]
fn retimed_synthetic_tone_video_companions_decode_the_same_pcm_as_ogg() {
    use fvid::playback_webm_audio::WebmAudioReader;
    for rate in [44100, 32000] {
        let bytes = fixture(&format!("vorbis-rate-tone-{rate}.ogg"));
        let mut baseline =
            fvid_media::owned_ogg_vorbis::Program::open(bytes.as_slice(), Default::default())
                .unwrap();
        let mut expected = Vec::new();
        baseline.decode_pcm(&mut expected).unwrap();
        let mut stream = WebmAudioReader::open(
            std::io::Cursor::new(fixture(&format!("hevc-vorbis-rate-tone-{rate}.mkv"))),
            Default::default(),
        )
        .unwrap();
        let mut decoder = stream.make_decoder().unwrap();
        let mut actual = Vec::new();
        while let Some(packet) = stream.next_packet().unwrap() {
            if let Some(decoded) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(pcm) = stream
                    .present_decoded(decoded.packet, decoded.source_pts)
                    .unwrap()
                {
                    actual.extend(pcm.data);
                }
            }
        }
        assert_eq!(actual, expected, "synthetic video rate {rate}");
    }
}
