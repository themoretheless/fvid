use std::{io::Cursor, path::PathBuf};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-sample-descriptions")
            .join(name),
    )
    .unwrap()
}
#[test]
fn owned_media_reader_accepts_equivalent_stsd_and_selects_the_actual_packet_bytes() {
    for name in ["equivalent", "equivalent-fragmented"] {
        let mut reader = fvid_media::owned_mp4::Mp4Reader::open(
            Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
        )
        .unwrap();
        assert_eq!(reader.tracks()[0].samples.len(), 3);
        for i in 0..3 {
            let mut packet = Vec::new();
            reader.read_packet(0, i, &mut packet).unwrap();
            assert_eq!(packet, read(&format!("equivalent-frame-{i}.vp9")));
        }
    }
}
#[test]
fn equivalent_pcm_and_aac_export_the_intended_owned_waveform() {
    for name in ["equivalent-pcm", "equivalent-aac"] {
        let mut actual = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(read(&format!("{name}.mp4"))),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        let expected = read(&format!("{name}.f32le"));
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 1e-6, "{name}: {a} != {b}");
        }
    }
}

#[test]
fn both_indexed_apis_keep_descriptions_and_encoded_packets_across_native_codecs() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for name in [
        "mp4-sample-descriptions/changing.mp4",
        "mp4-sample-descriptions/changing-fragmented.mp4",
        "mp4-sample-descriptions/compact-pcm.mp4",
        "mp4-audio-reconfiguration/pcm-fragmented.mp4",
        "mp4-audio-reconfiguration/eac3.mp4",
        "mp4-aac-transitions/changes.mp4",
        "mp4-aac-transitions/fragmented.mp4",
        "mp4-adpcm-transitions/fragmented-2000.mp4",
        "mp4-mace-transitions/mixed-hybrid.mp4",
    ] {
        let data = std::fs::read(base.join(name)).unwrap();
        let mut media =
            fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&data), Default::default())
                .unwrap();
        let mut facade =
            fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(media.tracks().len(), facade.tracks().len());
        for t in 0..media.tracks().len() {
            assert_eq!(
                media.tracks()[t].sample_descriptions.len(),
                facade.tracks()[t].sample_descriptions.len()
            );
            assert_eq!(
                media.tracks()[t].samples.len(),
                facade.tracks()[t].samples.len()
            );
            for i in 0..media.tracks()[t].samples.len() {
                let a = media.tracks()[t].samples.get(i).unwrap();
                let b = facade.tracks()[t].samples.get(i).unwrap();
                assert_eq!(
                    (
                        a.description_index,
                        a.offset,
                        a.size,
                        a.dts,
                        a.pts,
                        a.duration
                    ),
                    (
                        b.description_index,
                        b.offset,
                        b.size,
                        b.dts,
                        b.pts,
                        b.duration
                    ),
                    "{name}/{t}/{i}"
                );
                {
                    let a = media.tracks()[t].description_for_sample(i).unwrap();
                    let b = facade.tracks()[t].description_for_sample(i).unwrap();
                    assert_eq!(
                        (
                            a.codec,
                            a.width,
                            a.height,
                            a.channels,
                            a.sample_rate,
                            a.bit_depth,
                            &a.configuration,
                            a.pixel_aspect
                        ),
                        (
                            b.codec,
                            b.width,
                            b.height,
                            b.channels,
                            b.sample_rate,
                            b.bit_depth,
                            &b.configuration,
                            b.pixel_aspect
                        )
                    );
                }
                let mut a = Vec::new();
                let mut b = Vec::new();
                media.read_packet(t, i, &mut a).unwrap();
                facade.read_packet(t, i, &mut b).unwrap();
                assert_eq!(a, b);
            }
        }
    }
}
#[test]
fn malformed_description_references_fail_for_their_specific_reason() {
    for label in ["equivalent", "changing"] {
        for (suffix, expected) in [
            ("-chunk-0", "sample-to-chunk mapping"),
            ("-chunk-3", "sample-to-chunk mapping"),
            ("-fragment-3", "fragment sample description index"),
            ("-external-second", "invalid sample data reference"),
        ] {
            let error = fvid_media::owned_mp4::Mp4Reader::open_indexed(
                Cursor::new(read(&format!("{label}{suffix}.mp4"))),
                Default::default(),
            )
            .err()
            .unwrap()
            .to_string();
            assert!(error.contains(expected), "{label}{suffix}: {error}");
        }
    }
}
#[test]
fn fixed_setup_reader_and_mux_refuse_real_configuration_changes_before_output() {
    let data = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-aac-transitions/changes.mp4"),
    )
    .unwrap();
    let error = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default())
        .err()
        .unwrap();
    assert!(error.is_unsupported());
    assert!(error.to_string().contains("decoder reconfiguration"));
    let mut reader =
        fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&data), Default::default())
            .unwrap();
    let mut output = Cursor::new(Vec::new());
    let error = fvid_media::owned_mp4_matroska::write_selected(
        &mut reader,
        &mut output,
        None,
        None,
        None,
        &[1],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("decoder reconfiguration"), "{error}");
    assert!(output.into_inner().is_empty());
}
