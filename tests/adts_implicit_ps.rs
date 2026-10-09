use std::{io::Cursor, time::Duration};
#[test]
fn late_implicit_ps_adts_has_independent_stereo_pcm_and_exact_intervals() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/adts-implicit-ps.json"
    ))
    .unwrap();
    let gold = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for file in m["files"].as_array().unwrap() {
        let data = std::fs::read(root.join(file.as_str().unwrap())).unwrap();
        let mut indexed = fvid::container::mp4::Mp4Reader::open(
            std::fs::File::open(root.join(m["video"]["file"].as_str().unwrap())).unwrap(),
            Default::default(),
        )
        .unwrap();
        let audio = indexed
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let adts = fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default()).unwrap();
        assert_eq!(adts.packets(), 3);
        for i in 0..3 {
            let mut packet = Vec::new();
            indexed.read_packet(audio, i, &mut packet).unwrap();
            assert_eq!(packet, adts.packet(i));
        }
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, 2, 6144)
        );
        for (i, raw) in pcm.chunks_exact(4).enumerate() {
            let descriptor = &m["pcm"][i % 2];
            let at = descriptor[0].as_u64().unwrap() as usize + i / 2 * 8;
            let expected = f64::from_le_bytes(gold[at..at + 8].try_into().unwrap()) as f32;
            let actual = f32::from_le_bytes(raw.try_into().unwrap());
            assert!(
                (actual - expected).abs() <= 2. * f32::EPSILON * expected.abs() + 2e-16,
                "{} sample {i}",
                file
            );
        }
        let mut root_pcm = Vec::new();
        fvid::native_media::decode_aac_pcm_interval(
            &data,
            &mut root_pcm,
            &Default::default(),
            None,
        )
        .unwrap();
        assert_eq!(root_pcm, pcm);
        let mut selected = Vec::new();
        fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut selected,
            Some((Duration::from_millis(20), Duration::from_millis(110))),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(selected, pcm[960 * 8..5280 * 8]);
    }
}

#[cfg(feature = "player")]
#[test]
fn implicit_ps_player_rewind_seek_and_eof_drain_preserve_stereo() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
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
        output
    }
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/adts-implicit-ps.json"
    ))
    .unwrap();
    for file in m["files"].as_array().unwrap() {
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(file.as_str().unwrap()),
        )
        .unwrap();
        let mut expected = Vec::new();
        fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut reader =
            fvid::playback_aac::AacAudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(
            (reader.sample_rate(), reader.channels(), reader.timescale()),
            (48000, 2, 48000)
        );
        assert_eq!(reader.duration(), Some(Duration::from_millis(128)));
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 4300, 6144] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * 8..]);
        }
    }
}

#[test]
fn implicit_ps_packet_limits_negotiate_only_the_selected_prefix() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/adts-implicit-ps.json"
    ))
    .unwrap();
    for file in m["files"].as_array().unwrap() {
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(file.as_str().unwrap()),
        )
        .unwrap();
        for (limit, channels, samples) in [(1, 1, 2048), (2, 2, 4096), (3, 2, 6144)] {
            let mut output = Vec::new();
            let options = fvid_control::CopyOptions {
                max_packets: Some(limit),
                ..Default::default()
            };
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut output,
                None,
                &options,
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (48000, channels, samples)
            );
            assert_eq!(output.len(), samples as usize * channels as usize * 4);
        }
    }
}
