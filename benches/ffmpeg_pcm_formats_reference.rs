//! Explicit QuickTime and synthetic Matroska PCM reference comparisons.
mod mp4 {
use fvid::native_export::export_audio_pcm_selected as export;
fn mp4_pcm_tracks_export_intervals_and_match_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = std::env::temp_dir().join(format!("fvid-mov-pcm-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let mut count = 0;
    for name in ["pcm-tags.mov", "pcm-screen.mov"] {
        let source = root.join("tests/fixtures/audio").join(name);
        let reader = fvid::container::mp4::Mp4Reader::open(
            std::io::BufReader::new(std::fs::File::open(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        for (index, track) in reader.tracks().iter().enumerate() {
            if !matches!(
                &track.codec,
                b"sowt" | b"twos" | b"fl32" | b"fl64" | b"in24" | b"in32"
            ) {
                continue;
            }
            count += 1;
            let isolated = directory.join(format!("isolated-{name}-{index}.mov"));
            let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
            let result = std::process::Command::new(&binary)
                .args(["-v", "error", "-i"])
                .arg(&source)
                .args(["-map", &format!("0:i:{}", track.id), "-c", "copy"])
                .arg(&isolated)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let source = isolated;
            let index = 0;

            let output = directory.join(format!("{name}-{}.f32le", track.id));
            let stats = export(
                &source,
                &output,
                None,
                1.0,
                None,
                None,
                Some(index),
                None,
                None,
            )
            .unwrap();
            let bytes = std::fs::read(&output).unwrap();
            assert!(fvid::native_media::is_owned_audio_source(&source).unwrap());
            let plan = fvid::native_plan::decode_audio(&source, &Default::default()).unwrap();
            assert!(plan.steps[0].detail.contains("PCM"));
            assert!(plan.graph.is_none());
            #[cfg(feature = "media")]
            {
                let api = directory.join(format!("api-{name}-{}.f32le", track.id));
                fvid::media::decode_audio(&source, &api, &Default::default()).unwrap();
                assert_eq!(std::fs::read(api).unwrap(), bytes);
            }

            assert_eq!(
                bytes.len() as u64,
                stats.sample_frames * u64::from(stats.channels) * 4
            );
            assert!(
                fvid::native_media::audio_source_info_selected(&source, Some(index))
                    .unwrap()
                    .codec
                    .starts_with("pcm_")
            );
            let from = std::time::Duration::from_millis(1);
            let to = std::time::Duration::from_millis(3);
            let window = directory.join(format!("window-{name}-{}.f32le", track.id));
            export(
                &source,
                &window,
                Some((from, to)),
                0.5,
                None,
                None,
                Some(index),
                None,
                None,
            )
            .unwrap();
            let start =
                (from.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
            let end =
                (to.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
            let stride = usize::from(stats.channels) * 4;
            let expected: Vec<u8> = bytes[start * stride..end * stride]
                .as_chunks::<4>().0.iter()
                .flat_map(|b| (f32::from_le_bytes(*b) * 0.5).to_le_bytes())
                .collect();
            assert_eq!(std::fs::read(window).unwrap(), expected);
            if let Some(binary) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
                let reference = std::process::Command::new(binary)
                    .args(["-v", "error", "-i"])
                    .arg(&source)
                    .args(["-map", &format!("0:{index}"), "-f", "f32le", "-"])
                    .output()
                    .unwrap();
                assert!(
                    reference.status.success(),
                    "{}",
                    String::from_utf8_lossy(&reference.stderr)
                );
                assert_eq!(bytes, reference.stdout, "{name} track {index}");
            }
        }
    }
    assert!(count >= 4, "exercise all indexed PCM tracks");
}

fn quicktime_integer_and_float_endian_metadata_matches_reference() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let directory = std::env::temp_dir().join(format!("fvid-enda-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/pcm-tags.mov");
    for codec in [
        "pcm_u8",
        "pcm_s24le",
        "pcm_s24be",
        "pcm_s32le",
        "pcm_s32be",
        "pcm_f32le",
        "pcm_f32be",
        "pcm_f64le",
        "pcm_f64be",
    ] {
        let input = directory.join(format!("{codec}.mov"));
        let result = std::process::Command::new(&binary)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(["-map", "0:0", "-c:a", codec])
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = directory.join(format!("{codec}.f32le"));
        export(&input, &output, None, 1.0, None, None, None, None, None).unwrap();
        let reference = std::process::Command::new(&binary)
            .args(["-v", "error", "-i"])
            .arg(&input)
            .args(["-f", "f32le", "-"])
            .output()
            .unwrap();
        assert!(reference.status.success());
        assert_eq!(std::fs::read(output).unwrap(), reference.stdout, "{codec}");
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
                std::fs::File::open(&input).unwrap(),
                fvid::container::mp4::Limits::default(),
            )
            .unwrap();
            let mut decoder = fvid::codec::make_audio_decoder(
                reader.codec(),
                reader.extra_data(),
                reader.sample_rate(),
                reader.channels(),
                reader.bits_per_sample(),
            )
            .unwrap();
            let mut pcm = Vec::new();
            while let Some(packet) = reader.next_packet().unwrap() {
                if let Some(decoded) = decoder
                    .decode_encoded(
                        &packet.data,
                        packet.pts.max(0) as u64,
                        packet.duration.max(0) as u64,
                    )
                    .unwrap()
                {
                    pcm.extend_from_slice(&decoded.data);
                }
            }
            assert_eq!(pcm, reference.stdout, "player {codec}");
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

pub fn run() { mp4_pcm_tracks_export_intervals_and_match_reference(); quicktime_integer_and_float_endian_metadata_matches_reference(); }
}
mod matroska {
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
        if let Some(binary) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
            let result = std::process::Command::new(binary)
                .args(["-v", "error", "-i"])
                .arg(&source)
                .args(["-f", "f32le", "-"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(result.stdout, baseline, "{codec} {bits}");
        }
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
            .as_chunks::<8>().0.iter()
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

pub fn run() { owned_matroska_pcm_formats_intervals_and_failures(); }
}
fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    mp4::run(); matroska::run();
    println!("QuickTime and Matroska PCM format reference comparisons passed");
}
