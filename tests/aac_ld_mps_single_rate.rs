use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-single-rate")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn check(dual_rate: bool) {
    let manifest: Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        if (c["bands"].as_u64().unwrap() == 64) != dual_rate {
            continue;
        }
        let mut actual = Vec::new();
        let info = fvid::native_media::decode_mp4_aac_pcm(
            &read(c["video"].as_str().unwrap()),
            &mut actual,
        )
        .unwrap();
        assert_eq!(info.sample_frames, c["samples"].as_u64().unwrap());
        let gold = read(c["golden"].as_str().unwrap());
        assert_eq!(actual.len(), gold.len());
        let error = actual
            .chunks_exact(4)
            .zip(gold.chunks_exact(4))
            .map(|(a, b)| {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!(a.is_finite());
                (a - b).abs()
            })
            .fold(0.0f32, f32::max);
        assert!(
            error < 1e-7,
            "{}: shared LD-SBR synthesis PCM error {error}",
            c["name"]
        );
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(actual, mkv, "{} transport", c["name"]);
    }
}
#[test]
fn dual_rate_control_matches_complete_pcm() {
    check(true);
}
#[test]
fn single_rate_ld_sbr_uses_its_own_synthesis_convention() {
    check(false);
}

#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = vec![];
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    reader.validate_eof().unwrap();
    output
}
#[cfg(feature = "player")]
#[test]
fn shared_synthesis_player_rewind_and_sample_seek_preserve_complete_pcm() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        let mut readers: Vec<Box<dyn AudioStream>> = vec![
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(mp4),
                    Default::default(),
                )
                .unwrap(),
            ),
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(mkv),
                    Default::default(),
                )
                .unwrap(),
            ),
        ];
        for (container, reader) in readers.iter_mut().enumerate() {
            assert_eq!(play(reader.as_mut()), full, "{name} container {container}");
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full, "{name} rewind {container}");
            // Repeated targets and returning from EOF must rebuild the same
            // LD-SBR history as an uninterrupted decode, including frame edges.
            let mut targets = vec![
                -1,
                -c["container_frame_samples"].as_i64().unwrap(),
                0,
                13,
                512,
                2500,
                7,
                7,
                c["container_frame_samples"].as_i64().unwrap() - 1,
                c["container_frame_samples"].as_i64().unwrap(),
                c["container_frame_samples"].as_i64().unwrap() + 1,
                (full.len() / width) as i64,
                (full.len() / width) as i64 * 2,
                13,
                0,
            ];
            for edge in c["edges"].as_array().unwrap() {
                let edge = edge.as_i64().unwrap();
                targets.extend([edge - 1, edge, edge + 1]);
            }
            for target in targets {
                let ticks = (i128::from(target) * i128::from(reader.timescale())
                    / i128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                assert!(landed >= 0);
                if target <= (full.len() / width) as i64 {
                    assert_eq!(
                        landed,
                        ticks.max(0),
                        "{name} seek landed at a different target"
                    );
                }
                assert_eq!(
                    play(reader.as_mut()),
                    full[(((landed as u128 * u128::from(reader.sample_rate()))
                        .div_ceil(u128::from(reader.timescale()))
                        as usize)
                        * width)
                        .min(full.len())..],
                    "{name} container {container} seek {target} landed {landed}"
                );
            }
        }
    }
}

#[test]
fn shared_synthesis_convention_survives_checkpoint_reset_and_both_frontends() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder, aac_ld_mps_decor::Mode};
    let m: Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let blob = read("packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let movie = read(c["video"].as_str().unwrap());
        let reader =
            fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(movie), Default::default())
                .unwrap();
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .unwrap();
        let asc = fvid::codec::config::aac_specific_config(&track.configuration).unwrap();
        for mode in [Mode::Real, Mode::Complex] {
            let options = AacDecodeOptions {
                spatial_decorrelation: mode,
                ..Default::default()
            };
            let mut d = NativeAacDecoder::new_with_options(asc, options).unwrap();
            let mut frontend = fvid::codec::aac_native::NativeAacDecoder::new_with_options(
                asc,
                fvid::codec::aac_native::AacDecodeOptions {
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            for _ in 0..2 {
                let mut full = vec![];
                let frames = c["frames"].as_array().unwrap();
                let packet = |i: usize| {
                    let r = &frames[i];
                    let a = r["offset"].as_u64().unwrap() as usize;
                    &blob[a..a + r["bytes"].as_u64().unwrap() as usize]
                };
                for i in 0..3 {
                    let pcm = d.decode(packet(i)).unwrap();
                    assert_eq!(pcm, frontend.decode(packet(i)).unwrap());
                    full.extend(pcm);
                }
                let checkpoint = d.checkpoint();
                let mut tail = vec![];
                for i in 3..frames.len() {
                    let pcm = d.decode(packet(i)).unwrap();
                    assert_eq!(pcm, frontend.decode(packet(i)).unwrap());
                    tail.extend(pcm);
                }
                full.extend_from_slice(&tail);
                d.restore(&checkpoint).unwrap();
                let mut replay = vec![];
                for i in 3..frames.len() {
                    replay.extend(d.decode(packet(i)).unwrap());
                }
                assert_eq!(replay, tail);
                let actual: Vec<_> = full.iter().flat_map(|v| v.to_le_bytes()).collect();
                let gold = read(c["golden"].as_str().unwrap());
                assert_eq!(actual.len(), gold.len());
                let error = actual
                    .chunks_exact(4)
                    .zip(gold.chunks_exact(4))
                    .map(|(a, b)| {
                        (f32::from_le_bytes(a.try_into().unwrap())
                            - f32::from_le_bytes(b.try_into().unwrap()))
                        .abs()
                    })
                    .fold(0.0f32, f32::max);
                assert!(
                    error < 1e-7,
                    "{} mode {mode:?}: PCM error {error}",
                    c["name"]
                );
                d.reset();
                frontend.reset();
            }
        }
    }
}
