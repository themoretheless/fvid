use fvid::{
    codec::aac_native::NativeAacDecoder,
    container::adts::{Aac, Limits},
};
macro_rules! case {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(
                "fixtures/playback-errors/aac-height-",
                $name,
                ".aac"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "fixtures/playback-errors/aac-height-",
                $name,
                ".f32le"
            ))
            .as_slice(),
        )
    };
}
#[test]
fn height_layer_pcm_matches_independent_cosine_oracle_per_channel() {
    let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/aac-height-generated.json"
    ))
    .unwrap();
    for (name, data, oracle) in [
        case!("mono-top"),
        case!("mono-bottom"),
        case!("layered-front"),
        case!("layered-front-swap"),
        case!("top-front-pair"),
        case!("top-front-three"),
        case!("top-back-three"),
        case!("top-side"),
        case!("bottom-surround"),
        case!("top-wide"),
        case!("top-with-lfe"),
        case!("normal-four"),
        case!("multiple-lfe"),
        case!("normal-three-sce"),
        case!("normal-crc"),
        case!("short-comment"),
        case!("short-comment-two"),
    ] {
        let stream = Aac::parse(data, &Limits::default()).unwrap();
        let mut decoder = NativeAacDecoder::new(&stream.configuration).unwrap();
        assert_eq!(
            u64::from(decoder.channel_mask()),
            manifest["cases"][name]["mask"].as_u64().unwrap(),
            "{name}"
        );
        let mut actual = Vec::new();
        for index in 0..stream.packets() {
            actual.extend(decoder.decode(stream.packet(index)).unwrap());
        }
        assert_eq!(actual.len() * 4, oracle.len(), "{name}");
        let mut squared = 0.0f64;
        let mut peak = 0.0f64;
        for (&a, b) in actual.iter().zip(oracle.as_chunks::<4>().0) {
            let delta = f64::from(a) - f64::from(f32::from_le_bytes(*b));
            squared += delta * delta;
            peak = peak.max(delta.abs());
        }
        assert!(actual.iter().any(|v| v.abs() > 0.00001), "{name}");
        assert!(
            (squared / actual.len() as f64).sqrt() < 2e-7 && peak < 1e-6,
            "{name} RMS={} peak={peak}",
            (squared / actual.len() as f64).sqrt()
        );
        decoder.reset();
        assert_eq!(
            decoder.decode(stream.packet(0)).unwrap(),
            actual[..1024 * stream.channels as usize]
        );
    }
}

#[test]
fn explicit_positions_survive_wave_mask_limitations_and_pcm_reordering() {
    use fvid::codec::aac_pce::{ChannelPosition, HeightLayer as H, Position as P};
    let layered = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-height-layered-front.aac"),
        &Limits::default(),
    )
    .unwrap();
    let decoder = NativeAacDecoder::new(&layered.configuration).unwrap();
    assert_eq!(decoder.channel_mask(), 0);
    assert_eq!(
        decoder.channel_positions().unwrap().unwrap(),
        vec![
            ChannelPosition {
                height: H::Normal,
                position: P::Front,
                index: 0,
                group_channels: 1
            },
            ChannelPosition {
                height: H::Top,
                position: P::Front,
                index: 0,
                group_channels: 1
            },
            ChannelPosition {
                height: H::Bottom,
                position: P::Front,
                index: 0,
                group_channels: 1
            },
        ]
    );
    let top = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-height-top-front-three.aac"),
        &Limits::default(),
    )
    .unwrap();
    let decoder = NativeAacDecoder::new(&top.configuration).unwrap();
    assert_eq!(
        decoder
            .channel_positions()
            .unwrap()
            .unwrap()
            .iter()
            .map(|p| p.index)
            .collect::<Vec<_>>(),
        vec![1, 0, 2]
    );
    let bottom = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-height-bottom-surround.aac"),
        &Limits::default(),
    )
    .unwrap();
    let decoder = NativeAacDecoder::new(&bottom.configuration).unwrap();
    let positions = decoder.channel_positions().unwrap().unwrap();
    assert_eq!(
        (positions[0].height, positions[0].position),
        (H::Normal, P::Lfe)
    );
    assert!(positions[1..].iter().all(|p| p.height == H::Bottom));
    assert_eq!(
        positions.iter().map(|p| p.position).collect::<Vec<_>>(),
        vec![P::Lfe, P::Front, P::Side, P::Side, P::Back, P::Back]
    );
}

#[test]
fn crc_and_reserved_height_fail_transactionally_and_same_mask_layout_changes_are_rejected() {
    use fvid::codec::{aac_pce::ProgramConfig, bits::BitReader};
    for (data, message) in [
        (
            include_bytes!("fixtures/playback-errors/aac-height-invalid-crc.aac").as_slice(),
            "height CRC mismatch",
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-height-invalid-layer.aac").as_slice(),
            "invalid AAC PCE height layer",
        ),
    ] {
        assert!(
            Aac::parse(data, &Limits::default())
                .err()
                .unwrap()
                .to_string()
                .contains(message)
        );
        let mut bits = BitReader::new(&data[7..]);
        assert_eq!(bits.read(3).unwrap(), 5);
        assert!(
            ProgramConfig::read(&mut bits, 0)
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        assert_eq!(bits.position(), 3);
    }
    let original = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-height-layered-front.aac"),
        &Limits::default(),
    )
    .unwrap();
    let changed = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-height-layered-front-swap.aac"),
        &Limits::default(),
    )
    .unwrap();
    let mut actual = NativeAacDecoder::new(&original.configuration).unwrap();
    let mut expected = NativeAacDecoder::new(&original.configuration).unwrap();
    actual.decode(original.packet(0)).unwrap();
    expected.decode(original.packet(0)).unwrap();
    assert!(
        actual
            .decode(changed.packet(1))
            .unwrap_err()
            .to_string()
            .contains("changed the configured layout")
    );
    assert_eq!(
        actual.decode(original.packet(1)).unwrap(),
        expected.decode(original.packet(1)).unwrap()
    );
    let mut bits = BitReader::new(original.packet(0));
    bits.read(3).unwrap();
    let program = ProgramConfig::read(&mut bits, 0).unwrap();
    let end = bits.position();
    let (_, roundtrip) = fvid::codec::config::AacConfig::parse_with_program(
        &program.audio_specific_config().unwrap(),
    )
    .unwrap();
    assert_eq!(roundtrip.unwrap(), program);
    for cut in 1..end / 8 {
        let mut truncated = BitReader::new(&original.packet(0)[..cut]);
        truncated.read(3).unwrap();
        assert!(ProgramConfig::read(&mut truncated, 0).is_err(), "cut {cut}");
        assert_eq!(truncated.position(), 3);
    }
}

#[test]
fn height_wave_exports_preserve_pcm_and_do_not_invent_horizontal_masks() {
    let directory = std::env::temp_dir().join(format!("fvid-height-export-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    for (name, mask, channels) in [
        ("mono-top", 1 << 13, 1),
        ("top-front-three", (1 << 12) | (1 << 13) | (1 << 14), 3),
        ("bottom-surround", 0, 6),
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/aac-height-{name}.aac"
        ));
        let output = directory.join(format!("{name}.wav"));
        fvid::native_export::export_aac_pcm(&source, &output).unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let wave = fvid::native_pcm::inspect(&mut file, None).unwrap();
        assert_eq!(
            (wave.channels, wave.channel_mask, wave.sample_frames),
            (channels, mask, 6144)
        );
        let mut pcm = Vec::new();
        fvid::native_media::decode_aac_pcm(
            &std::fs::read(&source).unwrap(),
            &mut pcm,
            &Limits::default(),
        )
        .unwrap();
        assert!(std::fs::read(output).unwrap().ends_with(&pcm));
    }
    let input = include_bytes!("fixtures/playback-errors/aac-height-companion.y4m");
    let mut output = Vec::new();
    let stats = fvid::y4m::process(
        std::io::Cursor::new(input),
        &mut output,
        Default::default(),
        1 << 20,
    )
    .unwrap();
    assert_eq!(stats.frames, 6);
    assert_eq!(output, input);
}

#[test]
fn height_initialization_and_pcm_survive_owned_mp4_and_matroska_remux() {
    use fvid::container::{adts::StreamReader, matroska_write, mp4::Mp4Reader, mp4_write};
    for data in [
        include_bytes!("fixtures/playback-errors/aac-height-layered-front.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-height-top-front-three.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-height-bottom-surround.aac").as_slice(),
    ] {
        let indexed = Aac::parse(data, &Limits::default()).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_aac_pcm(data, &mut expected, &Limits::default()).unwrap();
        let mut mp4 = std::io::Cursor::new(Vec::new());
        mp4_write::write_adts_aac(data, &mut mp4).unwrap();
        let mp4 = mp4.into_inner();
        let reader = Mp4Reader::open(std::io::Cursor::new(&mp4), Default::default()).unwrap();
        assert_eq!(
            fvid::codec::config::aac_specific_config(&reader.tracks()[0].configuration).unwrap(),
            indexed.configuration
        );
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm_interval(&mp4, &mut actual, None).unwrap();
        assert_eq!(actual, expected);
        let mut mka = std::io::Cursor::new(Vec::new());
        matroska_write::write_adts(StreamReader::open(data).unwrap(), &mut mka, None, None)
            .unwrap();
        let mut actual = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mka.into_inner(), &mut actual, None)
            .unwrap();
        assert_eq!(actual, expected);
    }
}
