use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifests() -> [serde_json::Value; 2] {
    [
        serde_json::from_str(include_str!("fixtures/playback-errors/adts-first-ps.json")).unwrap(),
        serde_json::from_str(include_str!(
            "fixtures/playback-errors/adts-implicit-ps.json"
        ))
        .unwrap(),
    ]
}
fn check_source_info(data: &[u8]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "fvid-ps-remux-info-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let _guard = Cleanup(path.clone());
    std::io::Write::write_all(&mut file, data).unwrap();
    drop(file);
    let info = fvid::native_media::aac_source_info(&path).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.channel_mask),
        (48000, 2, 3)
    );
}
#[test]
fn first_and_late_ps_remux_preserves_raw_packets_output_clock_and_full_stereo() {
    use fvid::container::{adts, matroska_write, mp4, mp4_write};
    for m in manifests() {
        for file in m["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(file.as_str().unwrap())).unwrap();
            let source = adts::Aac::parse(&data, &Default::default()).unwrap();
            let mut gold = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut gold,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (48000, 2, 6144)
            );
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
                let mut reader =
                    mp4::Mp4Reader::open(Cursor::new(output.get_ref()), Default::default())
                        .unwrap();
                assert_eq!(
                    (
                        reader.tracks()[0].sample_rate,
                        reader.tracks()[0].timescale,
                        reader.tracks()[0].duration
                    ),
                    (48000, 48000, 6144)
                );
                for i in 0..3 {
                    let mut packet = Vec::new();
                    reader.read_packet(0, i, &mut packet).unwrap();
                    assert_eq!(packet, source.packet(i));
                }
                check_source_info(output.get_ref());
                let mut actual = Vec::new();
                fvid::native_media::decode_mp4_aac_pcm(output.get_ref(), &mut actual).unwrap();
                assert_eq!(actual, gold, "{} MP4", file);
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
            assert_eq!(indexed.tracks[0].sample_rate, 48000);
            for i in 0..3 {
                assert_eq!(indexed.read_packet(i).unwrap(), source.packet(i));
            }
            check_source_info(output.get_ref());
            let mut actual = Vec::new();
            fvid::native_media::decode_matroska_aac_pcm_interval(
                output.get_ref(),
                &mut actual,
                None,
            )
            .unwrap();
            assert_eq!(actual, gold, "{} Matroska", file);
        }
    }
}
#[test]
fn first_packet_ps_video_and_adts_match_independent_stereo_pcm() {
    let m = &manifests()[0];
    let scalar = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    let mut expected = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::fs::File::open(root().join(m["video"]["file"].as_str().unwrap())).unwrap(),
        &mut expected,
        None,
        &Default::default(),
    )
    .unwrap();
    for (i, b) in expected.chunks_exact(4).enumerate() {
        let at = m["pcm"][i % 2][0].as_u64().unwrap() as usize + i / 2 * 8;
        let gold = f64::from_le_bytes(scalar[at..at + 8].try_into().unwrap()) as f32;
        let actual = f32::from_le_bytes(b.try_into().unwrap());
        assert!((gold - actual).abs() <= 2. * f32::EPSILON * gold.abs() + 2e-16);
    }
    for file in m["files"].as_array().unwrap() {
        let mut actual = Vec::new();
        fvid_media::owned_aac::decode_adts_pcm(
            std::fs::File::open(root().join(file.as_str().unwrap())).unwrap(),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}
#[test]
fn implicit_ps_source_info_reports_decoded_stereo_geometry() {
    for m in manifests() {
        let info =
            fvid::native_media::aac_source_info(&root().join(m["video"]["file"].as_str().unwrap()))
                .unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.channel_mask),
            (48000, 2, 3)
        );
    }
}
