use fvid::native_export::export_audio_pcm_selected as export;
fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
    [id, &(body.len() as u32 | 0x1000_0000).to_be_bytes(), body].concat()
}
fn container(codec: &str, bits: u8, data: &[u8]) -> Vec<u8> {
    let audio = element(
        &[0xe1],
        &[
            element(&[0xb5], &48000f64.to_be_bytes()),
            element(&[0x9f], &[2]),
            element(&[0x62, 0x64], &[bits]),
        ]
        .concat(),
    );
    let track = element(
        &[0xae],
        &[
            element(&[0xd7], &[1]),
            element(&[0x83], &[2]),
            element(&[0x86], codec.as_bytes()),
            audio,
        ]
        .concat(),
    );
    let cluster = element(
        &[0x1f, 0x43, 0xb6, 0x75],
        &[
            element(&[0xe7], &[0]),
            element(&[0xa3], &[&[0x81, 0, 0, 0x80][..], data].concat()),
        ]
        .concat(),
    );
    [
        element(
            &[0x1a, 0x45, 0xdf, 0xa3],
            &element(&[0x42, 0x82], b"matroska"),
        ),
        element(
            &[0x18, 0x53, 0x80, 0x67],
            &[element(&[0x16, 0x54, 0xae, 0x6b], &track), cluster].concat(),
        ),
    ]
    .concat()
}
struct Dir(std::path::PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn owned_matroska_pcm_formats_intervals_and_failures() {
    let d = Dir(std::env::temp_dir().join(format!("fvid-mka-pcm-{}", std::process::id())));
    std::fs::create_dir_all(&d.0).unwrap();
    for (codec, bits) in [
        ("A_PCM/INT/LIT", 8),
        ("A_PCM/INT/LIT", 16),
        ("A_PCM/INT/LIT", 24),
        ("A_PCM/INT/LIT", 32),
        ("A_PCM/INT/BIG", 16),
        ("A_PCM/INT/BIG", 24),
        ("A_PCM/INT/BIG", 32),
        ("A_PCM/FLOAT/IEEE", 32),
        ("A_PCM/FLOAT/IEEE", 64),
    ] {
        let expected = [-1.0f32, 0.5, 0.0, -0.5, 0.25, 0.75];
        let raw: Vec<u8> = expected
            .iter()
            .flat_map(|&s| {
                if codec == "A_PCM/FLOAT/IEEE" {
                    if bits == 32 {
                        s.to_le_bytes().to_vec()
                    } else {
                        (s as f64).to_le_bytes().to_vec()
                    }
                } else if bits == 8 {
                    vec![(s * 128.0 + 128.0) as u8]
                } else {
                    let n = (s as f64 * (1u64 << (bits - 1)) as f64) as i32;
                    let len = usize::from(bits) / 8;
                    if codec.ends_with("BIG") {
                        n.to_be_bytes()[4 - len..].to_vec()
                    } else {
                        n.to_le_bytes()[..len].to_vec()
                    }
                }
            })
            .collect();
        let source = d.0.join("input.data");
        std::fs::write(&source, container(codec, bits, &raw)).unwrap();
        assert!(fvid::native_media::is_owned_audio_source(&source).unwrap());
        let output =
            d.0.join(format!("{bits}-{}.f32le", codec.replace('/', "-")));
        let stats = export(&source, &output, None, 1.0, None, None, None, None, None).unwrap();
        assert_eq!(stats.sample_frames, 3);
        let baseline: Vec<u8> = expected.iter().flat_map(|s| s.to_le_bytes()).collect();

        let encoded = std::fs::read(&source).unwrap();
        let mut owned = Vec::new();
        let own_stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
            std::io::Cursor::new(&encoded),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, baseline);
        assert_eq!(
            (
                own_stats.sample_frames,
                own_stats.decoded_frames,
                own_stats.sample_rate,
                own_stats.channels
            ),
            (3, 1, 48000, 2)
        );
        let interval = Some((
            std::time::Duration::from_micros(21),
            std::time::Duration::from_micros(60),
        ));
        let mut own_window = Vec::new();
        fvid_media::owned_matroska_pcm::decode_matroska_pcm(
            std::io::Cursor::new(&encoded),
            &mut own_window,
            interval,
            &fvid_control::CopyOptions {
                streams: vec![0],
                max_packets: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(own_window, baseline[16..]);
        let mut refused = Vec::new();
        assert!(fvid_media::owned_matroska_pcm::decode_matroska_pcm(
            std::io::Cursor::new(&encoded),
            &mut refused,
            None,
            &fvid_control::CopyOptions {
                max_controlled_bytes: Some(1024),
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("controlled memory budget exceeded"));
        assert!(refused.is_empty());
        let mut admitted = Vec::new();
        let admitted_stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
            std::io::Cursor::new(&encoded),
            &mut admitted,
            None,
            &fvid_control::CopyOptions {
                max_controlled_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(admitted_stats, own_stats);
        assert_eq!(admitted, baseline);

        let plan =
            fvid::native_plan::decode_audio_selected(&source, &Default::default(), None).unwrap();
        assert!(plan.steps[0].detail.contains("PCM"));
        assert!(plan.graph.is_none());
        let sources = vec![source.clone(), source.clone()];
        let mixed = d.0.join("mix.wav");
        fvid::native_audio_mix::mix_audio(&sources, &mixed, &Default::default()).unwrap();
        let mixed_pcm = d.0.join("mix.f32le");
        export(&mixed, &mixed_pcm, None, 1.0, None, None, None, None, None).unwrap();
        assert_eq!(std::fs::read(&mixed_pcm).unwrap(), baseline);
        std::fs::remove_file(mixed).unwrap();
        std::fs::remove_file(mixed_pcm).unwrap();
        let merged = d.0.join("merge.wav");
        fvid::native_audio_mix::merge_audio(&sources, &merged).unwrap();
        let merged_pcm = d.0.join("merge.f32le");
        export(
            &merged,
            &merged_pcm,
            None,
            1.0,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let joined: Vec<u8> = baseline
            .chunks_exact(8)
            .flat_map(|f| f.iter().chain(f).copied())
            .collect();
        assert_eq!(std::fs::read(&merged_pcm).unwrap(), joined);
        std::fs::remove_file(merged).unwrap();
        std::fs::remove_file(merged_pcm).unwrap();

        assert_eq!(std::fs::read(&output).unwrap(), baseline);
        let window = d.0.join("window.f32le");
        export(
            &source,
            &window,
            Some((
                std::time::Duration::from_micros(21),
                std::time::Duration::from_micros(60),
            )),
            0.5,
            None,
            None,
            Some(0),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&window).unwrap(),
            expected[4..]
                .iter()
                .flat_map(|s| (s * 0.5).to_le_bytes())
                .collect::<Vec<_>>()
        );
        std::fs::remove_file(&window).unwrap();
        #[cfg(feature = "media")]
        {
            let api = d.0.join("api.f32le");
            fvid::media::decode_audio(&source, &api, &Default::default()).unwrap();
            assert_eq!(std::fs::read(&api).unwrap(), baseline);
            std::fs::remove_file(api).unwrap();
        }
        if bits > 8 {
            std::fs::write(&source, container(codec, bits, &raw[..raw.len() - 1])).unwrap();
            assert!(export(&source, &window, None, 1.0, None, None, None, None, None).is_err());
            assert!(!window.exists());
            let mut own_failed = Vec::new();
            assert!(fvid_media::owned_matroska_pcm::decode_matroska_pcm(
                std::io::Cursor::new(std::fs::read(&source).unwrap()),
                &mut own_failed,
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string()
            .contains("incomplete channel frame"));
            assert!(own_failed.is_empty());
        }
    }
    let source = d.0.join("nan.mka");
    std::fs::write(
        &source,
        container(
            "A_PCM/FLOAT/IEEE",
            32,
            &[f32::NAN.to_le_bytes(), 0f32.to_le_bytes()].concat(),
        ),
    )
    .unwrap();
    let output = d.0.join("nan.f32le");
    assert!(export(&source, &output, None, 1.0, None, None, None, None, None).is_err());
    assert!(!output.exists());
}

#[test]
fn library_pcm_packet_float_endianness_and_strict_samples_match_frontend() {
    for bits in [32, 64] {
        let mut front = fvid::codec::pcm_decoder::PcmDecoder::new(
            fvid::codec::pcm_decoder::PcmFormat::Float { bits },
            48000,
            1,
        )
        .unwrap();
        let mut own = fvid_media::owned_pcm_decoder::PcmDecoder::new(
            fvid_media::owned_pcm_decoder::PcmFormat::Float { bits },
            48000,
            1,
        )
        .unwrap();
        own.set_float_big_endian(true);
        let data: Vec<u8> = [-1.0f64, 0.25, 0.75]
            .iter()
            .flat_map(|&s| {
                if bits == 32 {
                    (s as f32).to_be_bytes().to_vec()
                } else {
                    s.to_be_bytes().to_vec()
                }
            })
            .collect();
        use fvid::audio::AudioDecode;
        let little: Vec<u8> = [-1.0f64, 0.25, 0.75]
            .iter()
            .flat_map(|&s| {
                if bits == 32 {
                    (s as f32).to_le_bytes().to_vec()
                } else {
                    s.to_le_bytes().to_vec()
                }
            })
            .collect();
        let packet = front.decode_encoded(&little, 7, 0).unwrap().unwrap();
        let own_bytes: Vec<u8> = own
            .decode_pcm(&data)
            .unwrap()
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(packet.data, own_bytes);
        assert_eq!((own.sample_rate(), own.channels()), (48000, 1));
        assert!(own
            .decode_pcm(&data[..data.len() - 1])
            .unwrap_err()
            .to_string()
            .contains("incomplete channel frame"));
        let nan = if bits == 32 {
            f32::NAN.to_be_bytes().to_vec()
        } else {
            f64::NAN.to_be_bytes().to_vec()
        };
        assert!(own
            .decode_pcm(&nan)
            .unwrap_err()
            .to_string()
            .contains("non-finite"));
    }
}

#[test]
fn synthetic_pcm32_and_float64_export_retains_precision_through_shared_timeline() {
    let dir = Dir(std::env::temp_dir().join(format!("fvid-pcm-precision-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for (name, expected) in [
        (
            "pcm32-precision-little.mka",
            [
                2147483647.0,
                -2147483647.0,
                16777217.0,
                -16777217.0,
                1.0,
                -1.0,
            ]
            .map(|v| v / 2147483648.0),
        ),
        (
            "pcm32-precision-big.mka",
            [
                2147483647.0,
                -2147483647.0,
                16777217.0,
                -16777217.0,
                1.0,
                -1.0,
            ]
            .map(|v| v / 2147483648.0),
        ),
        (
            "pcm64-precision.mka",
            [
                0.12345678901234567,
                -0.9876543210987654,
                1.0000000000000002,
                -1.0000000000000002,
                1e-100,
                -1e-100,
            ],
        ),
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name);
        let encoded = std::fs::read(&source).unwrap();
        assert!(expected.iter().any(|&v| v != v as f32 as f64));
        let baseline: Vec<u8> = expected.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut precise = Vec::new();
        let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
            std::io::Cursor::new(&encoded),
            &mut precise,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (
                stats.sample_frames,
                stats.decoded_frames,
                stats.channels,
                stats.sample_rate
            ),
            (3, 1, 2, 48000)
        );
        assert_eq!(precise, baseline);
        let mut old = Vec::new();
        fvid_media::owned_matroska_pcm::decode_matroska_pcm(
            std::io::Cursor::new(&encoded),
            &mut old,
            None,
            &Default::default(),
        )
        .unwrap();
        let widened: Vec<u8> = old
            .chunks_exact(4)
            .flat_map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap())).to_le_bytes())
            .collect();
        assert_ne!(widened, baseline, "fixture must detect an f32 intermediate");
        let destination = dir.0.join(format!("{name}.wav"));
        let written = fvid_media::decode_audio(&source, &destination, &Default::default()).unwrap();
        assert_eq!((written.sample_frames, written.decoded_frames), (3, 1));
        let wave = std::fs::read(&destination).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut std::io::Cursor::new(&wave), None)
            .unwrap();
        assert!(info.float);
        assert_eq!(info.bits_per_sample, 64);
        assert_eq!(
            &wave[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
            baseline
        );
        let wave_source = dir.0.join(format!("{name}-reference.wav"));
        fvid_media::owned_wav_file::write_wav_f64le(&wave_source, 48000, 2, &expected).unwrap();
        let direct = dir.0.join(format!("{name}-dsp.wav"));
        let reference = dir.0.join(format!("{name}-dsp-reference.wav"));
        let transform = fvid_media_info::AudioDecodeTransform {
            volume: Some(0.25),
            channels: Some(1),
            sample_rate: Some(44100),
            ..Default::default()
        };
        fvid_media::owned_audio_export::decode_audio_transformed(
            &wave_source,
            &reference,
            transform,
            &Default::default(),
        )
        .unwrap();
        fvid_media::decode_audio_transformed(&source, &direct, transform, &Default::default())
            .unwrap();
        assert_eq!(
            std::fs::read(direct).unwrap(),
            std::fs::read(reference).unwrap(),
            "{name}: full f64 DSP pipeline differs from exact WAVE source"
        );
        let trimmed = dir.0.join(format!("{name}-trim.wav"));
        fvid_media::owned_audio_export::decode_audio_transformed(
            &source,
            &trimmed,
            fvid_media_info::AudioDecodeTransform {
                interval: Some((21, 60)),
                volume: Some(0.5),
                ..Default::default()
            },
            &fvid_control::CopyOptions {
                streams: vec![0],
                max_packets: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let wave = std::fs::read(&trimmed).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut std::io::Cursor::new(&wave), None)
            .unwrap();
        assert_eq!(info.sample_frames, 1);
        let window: Vec<u8> = expected[4..]
            .iter()
            .flat_map(|v| (v * 0.5).to_le_bytes())
            .collect();
        assert_eq!(
            &wave[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
            window
        );
    }
}
