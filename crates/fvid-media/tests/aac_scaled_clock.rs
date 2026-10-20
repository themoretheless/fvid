use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn same(actual: &[f32], expected: &[f32], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}");
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 2e-6, "{label}: {a}/{b}");
    }
}
fn floats(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn scaled_aac_library_and_public_export_keep_exact_samples_gaps_and_intervals() {
    let root = root();
    let base = root.join("aac-scaled-clock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let source = floats(&std::fs::read(root.join("aac-height-normal-ten.f32le")).unwrap());
    let stereo =
        floats(&std::fs::read(root.join("aac-device-layout/normal-ten.stereo.f32le")).unwrap());
    let dir = std::env::temp_dir().join(format!("fvid-aac-scaled-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let source = if let Some(control) = case["control"].as_str() {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(std::fs::read(root.join(control)).unwrap()),
                &mut pcm,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, case["coded_samples"].as_u64().unwrap());
            assert_eq!(usize::from(stats.channels), channels);
            floats(&pcm)
        } else {
            source.clone()
        };
        let stereo = if channels == 2 {
            source.clone()
        } else if case["control"].is_string() {
            let k = std::f32::consts::FRAC_1_SQRT_2;
            source
                .chunks_exact(10)
                .flat_map(|p| {
                    [
                        k * (p[0] + p[2] + p[4] + p[6]) + p[8],
                        k * (p[1] + p[3] + p[5] + p[7]) + p[9],
                    ]
                })
                .collect()
        } else {
            stereo.clone()
        };
        let input = std::fs::read(base.join(name)).unwrap();
        let gap = case["gap_samples"].as_u64().unwrap() as usize;
        let preroll = case["preroll_samples"].as_u64().unwrap_or(0) as usize;
        let mut expected = source[preroll * channels..].to_vec();
        expected.splice(
            3072 * channels..3072 * channels,
            std::iter::repeat_n(0., gap * channels),
        );
        let mut expected_stereo = stereo[preroll * 2..].to_vec();
        expected_stereo.splice(3072 * 2..3072 * 2, std::iter::repeat_n(0., gap * 2));
        for interval in [
            None,
            Some((Duration::from_millis(50), Duration::from_millis(70))),
            Some((Duration::from_millis(65), Duration::from_millis(70))),
        ] {
            let begin = interval.map_or(0, |(from, _)| {
                (from.as_nanos() * 48000 / 1_000_000_000) as usize
            });
            let end = interval.map_or(case["samples"].as_u64().unwrap() as usize, |(_, to)| {
                (to.as_nanos() * 48000 / 1_000_000_000) as usize
            });
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&input),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(stats.sample_frames, (end - begin) as u64, "{name}");
            same(
                &floats(&pcm),
                &expected[begin * channels..end * channels],
                name,
            );
        }
        let mut prefix = Vec::new();
        let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&input),
            &mut prefix,
            None,
            &fvid_control::CopyOptions {
                max_packets: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        let prefix_frames = 2 * case["frame_samples"].as_u64().unwrap() as usize - preroll;
        assert_eq!(stats.sample_frames, prefix_frames as u64);
        let prefix_control = if case["profile"] == "ps" {
            // Unlike a truncated ADTS probe, this container explicitly signals
            // stereo PS before any packet. Keep that geometry in the control.
            let setup =
                fvid_media::owned_webm::WebmReader::open(Cursor::new(&input), Default::default())
                    .unwrap();
            let asc = &setup
                .tracks
                .iter()
                .find(|t| t.codec == "A_AAC")
                .unwrap()
                .codec_private;
            let mut ps =
                fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder::new(asc).unwrap();
            assert_eq!((ps.sample_rate(), ps.channels()), (48000, 2));
            let mut transport = fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(
                std::fs::read(root.join(case["control"].as_str().unwrap())).unwrap(),
            ))
            .unwrap();
            let mut pcm = Vec::new();
            let mut emitted = Vec::new();
            for _ in 0..2 {
                if let Some(frame) = ps
                    .decode(&transport.next_packet().unwrap().unwrap())
                    .unwrap()
                {
                    emitted.push(frame.frame_index);
                    pcm.extend(frame.pcm);
                }
            }
            while let Some(frame) = ps.finish().unwrap() {
                emitted.push(frame.frame_index);
                pcm.extend(frame.pcm);
            }
            assert_eq!(emitted, vec![0, 1]);
            pcm[preroll * channels..].to_vec()
        } else {
            source[..prefix_frames * channels].to_vec()
        };
        same(&floats(&prefix), &prefix_control, name);
        let out = dir.join(format!("{name}.wav"));
        let stats = fvid_media::decode_audio_transformed(
            &base.join(name),
            &out,
            fvid_media_info::AudioDecodeTransform {
                channels: Some(2),
                interval: Some((50000, 70000)),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 960);
        let mut file = std::fs::File::open(out).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut bytes = vec![0; info.data_bytes as usize];
        file.read_exact(&mut bytes).unwrap();
        same(&floats(&bytes), &expected_stereo[2400 * 2..3360 * 2], name);
        let out = dir.join(format!("{name}-budget.wav"));
        let error = fvid_media::decode_audio_transformed(
            &base.join(name),
            &out,
            Default::default(),
            &fvid_control::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("controlled memory budget"),
            "{name}: {error}"
        );
        assert!(!out.exists());
    }
}
#[test]
fn raw_scaled_block_grid_survives_incremental_indexing_and_lace_offsets() {
    let base = root().join("aac-scaled-clock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(base.join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        let mut known = std::collections::BTreeMap::new();
        loop {
            for (i, packet) in reader
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == 2)
            {
                let grid = reader.packet_timestamp_grid(i).unwrap();
                if let Some(previous) = known.insert(i, grid) {
                    assert_eq!(previous, grid);
                }
                let _ = packet;
            }
            if reader.fully_indexed() {
                break;
            }
            reader.scan_more().unwrap();
        }
        let audio: Vec<_> = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == 2)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            audio.len(),
            (case["coded_samples"].as_u64().unwrap() / case["frame_samples"].as_u64().unwrap())
                as usize
        );
        let factor = reader.track_timestamp_scale(2).unwrap();
        let declared = case["factor_be"].as_str().unwrap();
        assert_eq!(format!("{:016x}", factor.to_bits()), declared);
        for group in case["groups"].as_array().unwrap() {
            let first = group["first"].as_u64().unwrap() as usize;
            let count = group["count"].as_u64().unwrap() as usize;
            for offset in 0..count {
                let grid = reader.packet_timestamp_grid(audio[first + offset]).unwrap();
                assert_eq!(grid.cluster, group["cluster"].as_u64().unwrap());
                assert_eq!(
                    grid.relative,
                    i32::try_from(group["relative"].as_i64().unwrap()).unwrap()
                );
                assert_eq!(
                    grid.offset_ns,
                    (offset as u128
                        * (u128::from(case["frame_samples"].as_u64().unwrap()) * 1_000_000_000)
                            .div_ceil(48000)) as i128
                );
            }
        }
    }
    let mut plain = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(root().join("aac-device-layout/normal-ten.mkv")).unwrap()),
        Default::default(),
    )
    .unwrap();
    plain.scan_all().unwrap();
    assert!(
        plain
            .packets
            .iter()
            .enumerate()
            .all(|(i, _)| plain.packet_timestamp_grid(i).is_none())
    );
}
