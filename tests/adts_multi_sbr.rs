use std::{io::Cursor, time::Duration};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/adts-multi-sbr.json")).unwrap()
}
fn expected(c: &serde_json::Value) -> Vec<f32> {
    let gold = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
    let missing = include_bytes!("fixtures/playback-errors/he-aac-missing-sbr.f64le");
    let channels = c["channels"].as_u64().unwrap() as usize;
    let mut pcm = vec![0.; 6144 * channels];
    let mut source = 0;
    for (i, width) in c["widths"].as_array().unwrap().iter().enumerate() {
        for _ in 0..width.as_u64().unwrap() {
            let target = c["mapping"][source].as_u64().unwrap() as usize;
            source += 1;
            if let Some(off) = c["pcm_offsets"][i].as_u64() {
                let absent = c["missing_element"].as_u64() == Some(i as u64);
                let at = if absent {
                    c["missing_pcm_offset"].as_u64().unwrap()
                } else {
                    off
                };
                let reference: &[u8] = if absent { missing } else { gold };
                for n in 0..6144 {
                    let index = at as usize + n * 8;
                    pcm[n * channels + target] =
                        f64::from_le_bytes(reference[index..index + 8].try_into().unwrap()) as f32;
                }
            }
        }
    }
    assert_eq!(source, channels);
    pcm
}
#[test]
fn multi_element_adts_sbr_matches_independent_lanes_layout_clock_and_intervals() {
    for c in cases()["cases"].as_array().unwrap() {
        let gold = expected(c);
        let channels = c["channels"].as_u64().unwrap() as u16;
        for name in c["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(name.as_str().unwrap())).unwrap();
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
                (48000, channels, 6144)
            );
            assert_eq!(pcm.len(), gold.len() * 4);
            for (i, b) in pcm.chunks_exact(4).enumerate() {
                assert!(
                    (f32::from_le_bytes(b.try_into().unwrap()) - gold[i]).abs() < 1e-7,
                    "{} sample {i}",
                    name
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
            let stride = channels as usize * 4;
            assert_eq!(selected, pcm[960 * stride..5280 * stride]);
        }
    }
}

#[test]
fn multi_element_adts_sbr_remux_preserves_packets_geometry_and_full_pcm() {
    use fvid::container::{adts, matroska_write, mp4, mp4_write};
    for c in cases()["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as u16;
        let mut video = mp4::Mp4Reader::open(
            std::fs::File::open(root().join(c["video"]["file"].as_str().unwrap())).unwrap(),
            Default::default(),
        )
        .unwrap();
        let audio = video
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        for name in c["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(name.as_str().unwrap())).unwrap();
            let source = adts::Aac::parse(&data, &Default::default()).unwrap();
            assert_eq!(source.packets(), 3);
            for i in 0..3 {
                let mut p = Vec::new();
                video.read_packet(audio, i, &mut p).unwrap();
                assert_eq!(p, source.packet(i));
            }
            let mut gold = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut gold,
                None,
                &Default::default(),
            )
            .unwrap();
            for sequential in [false, true] {
                let mut output = Cursor::new(Vec::new());
                if sequential {
                    mp4_write::write_adts_aac_reader(
                        adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                        &mut output,
                    )
                    .unwrap();
                } else {
                    mp4_write::write_adts_aac(&data, &mut output).unwrap();
                }
                let mut indexed =
                    mp4::Mp4Reader::open(Cursor::new(output.get_ref()), Default::default())
                        .unwrap();
                assert_eq!(
                    (
                        indexed.tracks()[0].sample_rate,
                        indexed.tracks()[0].channels,
                        indexed.tracks()[0].duration
                    ),
                    (48000, channels, 6144)
                );
                for i in 0..3 {
                    let mut p = Vec::new();
                    indexed.read_packet(0, i, &mut p).unwrap();
                    assert_eq!(p, source.packet(i));
                }
                let mut actual = Vec::new();
                fvid::native_media::decode_mp4_aac_pcm(output.get_ref(), &mut actual).unwrap();
                assert_eq!(actual, gold);
            }
            let mut output = Cursor::new(Vec::new());
            matroska_write::write_adts(
                adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                &mut output,
                None,
                None,
            )
            .unwrap();
            let mut indexed = fvid::container::webm::WebmReader::open(
                Cursor::new(output.get_ref()),
                Default::default(),
            )
            .unwrap();
            indexed.scan_all().unwrap();
            assert_eq!(indexed.packets.len(), 3);
            assert_eq!(
                (indexed.tracks[0].sample_rate, indexed.tracks[0].channels),
                (48000, u64::from(channels))
            );
            for i in 0..3 {
                assert_eq!(indexed.read_packet(i).unwrap(), source.packet(i));
            }
            let mut actual = Vec::new();
            fvid::native_media::decode_matroska_aac_pcm_interval(
                output.get_ref(),
                &mut actual,
                None,
            )
            .unwrap();
            assert_eq!(actual, gold);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn multi_element_sbr_player_clock_rewind_and_seek_keep_all_lanes() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        let mut i = 0;
        while let Some(p) = reader.next_packet().unwrap() {
            assert_eq!((p.pts, p.duration), (i * 2048, 2048));
            i += 1;
            if let Some(frame) = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
            {
                if let Some(f) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    pcm.extend(f.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(f) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(f.data);
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        pcm
    }
    for c in cases()["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as u16;
        for name in c["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(name.as_str().unwrap())).unwrap();
            let mut gold = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut gold,
                None,
                &Default::default(),
            )
            .unwrap();
            let mut reader =
                fvid::playback_aac::AacAudioReader::open(Cursor::new(&data), Default::default())
                    .unwrap();
            assert_eq!(
                (reader.sample_rate(), reader.timescale(), reader.channels()),
                (48000, 48000, channels)
            );
            assert_eq!(reader.duration(), Some(Duration::from_millis(128)));
            assert_eq!(play(&mut reader), gold);
            reader.rewind();
            assert_eq!(play(&mut reader), gold);
            for target in [1100, 4300, 6144] {
                let landed = reader.seek_to(target);
                assert_eq!(
                    play(&mut reader),
                    gold[landed as usize * channels as usize * 4..]
                );
            }
        }
    }
}

#[test]
fn multichannel_sbr_budget_is_admitted_before_publishing_pcm() {
    let m = cases();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channels"] == 6)
        .unwrap();
    let data = std::fs::read(root().join(c["files"][0].as_str().unwrap())).unwrap();
    let options = fvid_control::CopyOptions {
        max_controlled_bytes: Some(10 * 1024 * 1024),
        ..Default::default()
    };
    let mut output = Vec::new();
    let error =
        fvid_media::owned_aac::decode_adts_pcm(Cursor::new(&data), &mut output, None, &options)
            .unwrap_err();
    assert!(error
        .to_string()
        .contains("controlled memory budget exceeded"));
    assert!(output.is_empty());
}
