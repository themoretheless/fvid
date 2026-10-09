use std::{io::Cursor, time::Duration};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/adts-pce-sbr.json")).unwrap()
}
#[test]
fn stereo_pce_sbr_adts_matches_independent_pcm_and_exact_interval() {
    let gold = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
    for c in cases()["cases"].as_array().unwrap() {
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
                (48000, 2, 6144)
            );
            for (i, b) in pcm.chunks_exact(4).enumerate() {
                let at = c["pcm_offset"].as_u64().unwrap() as usize + (i / 2) * 8;
                let expected = f64::from_le_bytes(gold[at..at + 8].try_into().unwrap()) as f32;
                assert!(
                    (f32::from_le_bytes(b.try_into().unwrap()) - expected).abs() < 1e-7,
                    "{} sample {i}",
                    name
                );
            }
            let mut selected = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut selected,
                Some((Duration::from_millis(20), Duration::from_millis(110))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(selected, pcm[960 * 8..5280 * 8]);
            let mut root_pcm = Vec::new();
            fvid::native_media::decode_aac_pcm_interval(
                &data,
                &mut root_pcm,
                &Default::default(),
                None,
            )
            .unwrap();
            assert_eq!(root_pcm, pcm);
        }
    }
}

#[test]
fn stereo_pce_sbr_remux_preserves_companion_packets_and_pcm() {
    use fvid::container::{adts, matroska_write, mp4, mp4_write};
    for c in cases()["cases"].as_array().unwrap() {
        let mut companion = mp4::Mp4Reader::open(
            std::fs::File::open(root().join(c["video"]["file"].as_str().unwrap())).unwrap(),
            Default::default(),
        )
        .unwrap();
        let track = companion
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
                companion.read_packet(track, i, &mut p).unwrap();
                assert_eq!(p, source.packet(i));
            }
            let mut expected = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut expected,
                None,
                &Default::default(),
            )
            .unwrap();
            for sequential in [false, true] {
                let mut out = Cursor::new(Vec::new());
                if sequential {
                    mp4_write::write_adts_aac_reader(
                        adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                        &mut out,
                    )
                    .unwrap();
                } else {
                    mp4_write::write_adts_aac(&data, &mut out).unwrap();
                }
                let mut indexed =
                    mp4::Mp4Reader::open(Cursor::new(out.get_ref()), Default::default()).unwrap();
                assert_eq!(
                    (
                        indexed.tracks()[0].sample_rate,
                        indexed.tracks()[0].timescale,
                        indexed.tracks()[0].duration
                    ),
                    (48000, 48000, 6144)
                );
                for i in 0..3 {
                    let mut p = Vec::new();
                    indexed.read_packet(0, i, &mut p).unwrap();
                    assert_eq!(p, source.packet(i));
                }
                let mut actual = Vec::new();
                fvid::native_media::decode_mp4_aac_pcm(out.get_ref(), &mut actual).unwrap();
                assert_eq!(actual, expected);
            }
            let mut out = Cursor::new(Vec::new());
            matroska_write::write_adts(
                adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                &mut out,
                None,
                None,
            )
            .unwrap();
            let mut indexed = fvid::container::webm::WebmReader::open(
                Cursor::new(out.get_ref()),
                Default::default(),
            )
            .unwrap();
            indexed.scan_all().unwrap();
            assert_eq!(indexed.tracks[0].sample_rate, 48000);
            assert_eq!(indexed.packets.len(), 3);
            for i in 0..3 {
                assert_eq!(indexed.read_packet(i).unwrap(), source.packet(i));
            }
            let mut actual = Vec::new();
            fvid::native_media::decode_matroska_aac_pcm_interval(out.get_ref(), &mut actual, None)
                .unwrap();
            assert_eq!(actual, expected);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn stereo_pce_sbr_playback_has_output_clock_rewind_and_seek() {
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
        for name in c["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(name.as_str().unwrap())).unwrap();
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
                (reader.sample_rate(), reader.timescale(), reader.channels()),
                (48000, 48000, 2)
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
}
