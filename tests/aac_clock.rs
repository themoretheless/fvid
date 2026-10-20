#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::io::Cursor;
#[test]
fn quantized_aac_clocks_preserve_samples_and_real_discontinuities() {
    for (name, expected) in [
        (
            "coarse",
            vec![0, 21333333, 42666666, 64000000, 85333333, 106666666],
        ),
        (
            "tick-gap",
            vec![
                0,
                21_333_333,
                42_666_666,
                65_000_000,
                86_333_333,
                107_666_666,
            ],
        ),
        (
            "jumps",
            vec![0, 21333333, 142000000, 163333333, 10000000, 31333333],
        ),
    ] {
        let bytes = std::fs::read(format!(
            "{}/tests/fixtures/playback-errors/aac-clock-{name}.mkv",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 6);
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        for _ in 0..2 {
            let mut decoder = reader.make_decoder().unwrap();
            let mut times = Vec::new();
            let mut pcm = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                if let Some(frame) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    if let Some(out) = reader.present_decoded(frame.packet, p.pts).unwrap() {
                        times.push(out.pts);
                        pcm.extend(out.data);
                    }
                }
            }
            assert_eq!(times, expected, "{name}");
            let end = match name {
                "coarse" => 128_000_000,
                "tick-gap" => 129_000_000,
                _ => 184_666_666,
            };
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(end)),
                "{name} endpoint"
            );
            reader.rewind();
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(end)),
                "{name} stable endpoint"
            );
            let oracle = std::fs::read(format!(
                "{}/tests/fixtures/playback-errors/aac-height-normal-ten.f32le",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            assert_eq!(pcm.len(), oracle.len());
            for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
                assert!(
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                        < 1e-6
                );
            }
            reader.rewind();
        }
        if name == "jumps" {
            reader.seek_to(150_000_000);
            assert!(reader.preroll_target().is_none());
            assert!(!reader.resume_preroll(0));
        }
    }
}

#[test]
fn quantized_aac_seek_trims_at_exact_requested_sample() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let oracle = std::fs::read(root.join("aac-height-normal-ten.f32le")).unwrap();
    for (target, first) in [(50_000_000, 2400usize), (50_000_001, 2401), (0, 0)] {
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(std::fs::read(root.join("aac-clock-coarse.mkv")).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(reader.seek_to(target), target);
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        let mut first_pts = None;
        while let Some(p) = reader.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                .unwrap()
            {
                if let Some(out) = reader.present_decoded(frame.packet, p.pts).unwrap() {
                    first_pts.get_or_insert(out.pts);
                    pcm.extend(out.data);
                }
            }
        }
        assert_eq!(first_pts, Some(first as u64 * 1_000_000_000 / 48000));
        let expected = &oracle[first * 10 * 4..];
        assert_eq!(pcm.len(), expected.len());
        for (a, b) in pcm.chunks_exact(4).zip(expected.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-6
            );
        }
    }
}

#[test]
fn warmed_aac_seek_restores_the_paired_quantized_sample_clock() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(std::fs::read(root.join("aac-clock-long.mkv")).unwrap()),
        Default::default(),
    )
    .unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root.join("aac-clock-long.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    let mut video_frames = 0;
    while video.read_frame_raw().unwrap().is_some() {
        video_frames += 1;
    }
    assert_eq!(video_frames, 72);
    let mut decoder = reader.make_decoder().unwrap();
    let mut full = Vec::new();
    let mut saved = None;
    let mut index = 0;
    while let Some(p) = reader.next_packet().unwrap() {
        // The real worker stores the first decoder checkpoint in each second,
        // immediately after reading that packet and before decoding it.
        if index == 47 {
            saved = Some((p.pts, decoder.checkpoint().unwrap()));
        }
        if let Some(frame) = decoder
            .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
            .unwrap()
        {
            if let Some(out) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                full.extend(out.data);
            }
        }
        index += 1;
    }
    assert_eq!(index, 72);
    assert_eq!(full.len(), 72 * 1024 * 10 * 4);
    let (pts, state) = saved.unwrap();
    assert_eq!(pts, 1_002_000_000);
    assert_eq!(reader.seek_to(1_300_000_000), 1_300_000_000);
    assert!(reader.resume_preroll(pts));
    decoder.restore(&state).unwrap();
    let mut tail = Vec::new();
    let mut first = None;
    while let Some(p) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
            .unwrap()
        {
            if let Some(out) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                first.get_or_insert(out.pts);
                tail.extend(out.data);
            }
        }
    }
    assert_eq!(first, Some(1_300_000_000));
    assert_eq!(tail.len(), full.len() - 62400 * 10 * 4);
    assert!(
        tail == full[62400 * 10 * 4..],
        "warm seek PCM differs from uninterrupted decode"
    );
    assert_eq!(
        reader.duration(),
        Some(std::time::Duration::from_millis(1536))
    );
}

#[test]
fn aac_packet_tail_padding_trims_only_its_source_pcm_and_endpoint() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let bytes = std::fs::read(root.join("aac-clock-tail.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    let mut count = 0;
    while video.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 6);
    let mut reader =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    for _ in 0..2 {
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            if let Some(f) = decoder
                .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                .unwrap()
            {
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    pcm.extend(out.data);
                }
            }
        }
        let oracle = std::fs::read(root.join("aac-height-normal-ten.f32le")).unwrap();
        assert_eq!(pcm.len(), (6144 - 13) * 10 * 4);
        for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-6
            );
        }
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_nanos(
                6131 * 1_000_000_000 / 48000
            ))
        );
        reader.rewind();
    }
}

#[test]
fn delayed_ps_padding_uses_original_source_metadata_including_eof() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for name in ["immediate", "late"] {
        let adts =
            std::fs::read(root.join(format!("aac-main-ps-adts-{name}-synthetic.aac"))).unwrap();
        let mut reference = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(adts),
            &mut reference,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!((stats.channels, stats.sample_frames), (2, 24576));
        let width = 8;
        let mut expected = reference[..(24576 - 2048 - 7) * width].to_vec();
        expected.extend_from_slice(&reference[(24576 - 2048) * width..(24576 - 13) * width]);
        let bytes = std::fs::read(root.join(format!("aac-clock-ps-{name}-tail.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 12);
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        for _ in 0..2 {
            let mut decoder = reader.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut returned = 0;
            while let Some(p) = reader.next_packet().unwrap() {
                if let Some(f) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        actual.extend(out.data);
                        returned += 1;
                    }
                }
            }
            while let Some(f) = decoder.finish_packet().unwrap() {
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    actual.extend(out.data);
                    returned += 1;
                }
            }
            assert_eq!(returned, 12);
            assert_eq!(actual.len(), expected.len(), "{name}");
            for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
                assert!(
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                        < 1e-6,
                    "{name}"
                );
            }
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(
                    (24576 - 13) * 1_000_000_000 / 48000
                ))
            );
            reader.rewind();
        }
    }
}

#[test]
fn aac_padding_larger_than_its_decoded_packet_is_explicitly_refused() {
    let bytes = std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/aac-clock-invalid-tail.mkv",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    let mut count = 0;
    while video.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 6);
    let mut reader =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = reader.make_decoder().unwrap();
    let mut presented = 0;
    while let Some(p) = reader.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
            .unwrap()
            .unwrap();
        match reader.present_decoded(f.packet, f.source_pts) {
            Ok(Some(_)) => presented += 1,
            Err(error) => {
                assert_eq!(presented, 5);
                assert!(
                    error
                        .to_string()
                        .contains("discard padding exceeds AAC packet")
                );
                return;
            }
            _ => panic!("unexpected preroll"),
        }
    }
    panic!("invalid tail padding was accepted");
}
