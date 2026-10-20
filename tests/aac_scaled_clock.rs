#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn scaled_aac_block_coordinates_recover_exact_pcm_clock_and_seek_windows() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let dir = root.join("aac-scaled-clock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let oracle = std::fs::read(root.join("aac-height-normal-ten.f32le")).unwrap();
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        if case["profile"] != "lc" || case["coded_samples"].as_u64().unwrap() != 6144 {
            continue;
        }
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut frames = 0;
        while video.read_frame_raw().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, case["video_frames"].as_u64().unwrap());
        for target in [0, 50_000_000, 50_000_001, 65_000_000] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            reader.seek_to(target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut pcm = Vec::new();
            let mut pts = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                let f = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                    .unwrap();
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    pts.push(out.pts);
                    pcm.extend(out.data);
                }
            }
            if target == 0 {
                assert_eq!(pcm.len(), oracle.len(), "{name} raw PCM");
                for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
                    assert!(
                        (f32::from_le_bytes(a.try_into().unwrap())
                            - f32::from_le_bytes(b.try_into().unwrap()))
                        .abs()
                            < 1e-6,
                        "{name} raw PCM"
                    );
                }
            }
            let requested = (target as u128 * 48000).div_ceil(1_000_000_000) as usize;
            let gap = case["gap_samples"].as_u64().unwrap() as usize;
            let mut expected_pts = Vec::new();
            let mut expected_pcm = Vec::new();
            for i in 0..6 {
                let start = i * 1024 + if i >= 3 { gap } else { 0 };
                let skip = requested.saturating_sub(start).min(1024);
                if skip < 1024 {
                    expected_pts.push((start + skip) as u64 * 1_000_000_000 / 48000);
                    expected_pcm
                        .extend_from_slice(&oracle[(i * 1024 + skip) * 40..(i + 1) * 1024 * 40]);
                }
            }
            if pts != expected_pts || pcm.len() != expected_pcm.len() {
                failures.push(format!(
                    "{name}/{target}: PTS {pts:?}, PCM frames {}",
                    pcm.len() / 40
                ));
                continue;
            }
            for (a, b) in pcm.chunks_exact(4).zip(expected_pcm.chunks_exact(4)) {
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
                    case["samples"].as_u64().unwrap() * 1_000_000_000 / 48000
                ))
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn scaled_laced_main_ps_keeps_original_source_clock_through_delayed_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let base = root.join("aac-scaled-clock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["profile"] == "ps")
    {
        let name = case["file"].as_str().unwrap();
        let raw = std::fs::read(root.join(case["control"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(raw),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!((stats.channels, stats.sample_frames), (2, 24576));
        let bytes = std::fs::read(base.join(name)).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 4);
        for target in [0, 50_000_000, 50_000_001, 65_000_000] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            reader.seek_to(target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut pts = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                if let Some(f) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        pts.push(out.pts);
                        actual.extend(out.data);
                    }
                }
            }
            while let Some(f) = decoder.finish_packet().unwrap() {
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    pts.push(out.pts);
                    actual.extend(out.data);
                }
            }
            let preroll = case["preroll_samples"].as_u64().unwrap_or(0) as usize;
            let first = (target as u128 * 48000).div_ceil(1_000_000_000) as usize + preroll;
            let wanted: Vec<_> = (0..12)
                .filter_map(|i| {
                    let at = (i * 2048).max(first);
                    (at < (i + 1) * 2048).then_some((at - preroll) as u64 * 1_000_000_000 / 48000)
                })
                .collect();
            assert_eq!(pts, wanted, "{name}");
            assert_eq!(actual.len(), expected.len() - first * 8);
            for (a, b) in actual
                .chunks_exact(4)
                .zip(expected[first * 8..].chunks_exact(4))
            {
                assert!(
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                        < 1e-6,
                    "{name}/{target}"
                );
            }
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(
                    case["samples"].as_u64().unwrap() * 1_000_000_000 / 48000
                ))
            );
            reader.rewind();
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(
                    case["samples"].as_u64().unwrap() * 1_000_000_000 / 48000
                ))
            );
        }
    }
}

#[test]
fn scaled_warm_seek_restores_exact_grid_phase_and_paired_decoder_history() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-scaled-clock");
    let bytes = std::fs::read(root.join("warm-third.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    let mut count = 0;
    while video.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 72);
    let mut expected = Vec::new();
    let stats = fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(std::fs::read(root.join("warm-third.aac")).unwrap()),
        &mut expected,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!((stats.channels, stats.sample_frames), (10, 73728));
    let mut reader =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = reader.make_decoder().unwrap();
    let mut actual = Vec::new();
    let mut saved = None;
    let mut index = 0;
    while let Some(p) = reader.next_packet().unwrap() {
        if index == 47 {
            saved = Some((p.pts, decoder.checkpoint().unwrap()));
        }
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
            .unwrap()
            .unwrap();
        let out = reader
            .present_decoded(f.packet, f.source_pts)
            .unwrap()
            .unwrap();
        assert_eq!(out.pts, index * 1024 * 1_000_000_000 / 48000);
        actual.extend(out.data);
        index += 1;
    }
    assert_eq!(index, 72);
    assert!(
        actual == expected,
        "continuous scaled PCM differs from authored ADTS control"
    );
    let (pts, state) = saved.unwrap();
    assert_eq!(pts, 1_002_666_667);
    for target in [1_300_000_000, 1_300_000_001] {
        reader.seek_to(target);
        assert!(reader.resume_preroll(pts));
        decoder.restore(&state).unwrap();
        let mut tail = Vec::new();
        let mut first = None;
        while let Some(p) = reader.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                .unwrap()
                .unwrap();
            if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                first.get_or_insert(out.pts);
                tail.extend(out.data);
            }
        }
        let skip = (target as u128 * 48000).div_ceil(1_000_000_000) as usize;
        assert_eq!(first, Some(skip as u64 * 1_000_000_000 / 48000));
        assert_eq!(tail.len(), expected.len() - skip * 40);
        assert!(
            tail == expected[skip * 40..],
            "warm scaled seek PCM differs from continuous source"
        );
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_millis(1536))
        );
    }
}
