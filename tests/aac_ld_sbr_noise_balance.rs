use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-sbr-noise-balance")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn check(centered: bool) {
    let manifest: Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        if (c["mode"].as_str().unwrap() == "coupled") != centered {
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
            "{}: coupled noise balance PCM error {error}",
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
fn centered_noise_balance_control_matches_complete_pcm() {
    check(true);
}
#[test]
fn asymmetric_noise_balance_matches_normative_channel_orientation() {
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
fn noise_balance_player_rewind_and_sample_seek_preserve_normative_pcm() {
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
            let targets = vec![
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
