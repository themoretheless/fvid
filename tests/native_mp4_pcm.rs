use fvid::native_export::export_audio_pcm_selected as export;
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG to isolate saved MOV tracks"]
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
            if !matches!(&track.codec, b"sowt" | b"twos" | b"fl32" | b"fl64") {
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
                .chunks_exact(4)
                .flat_map(|b| (f32::from_le_bytes(b.try_into().unwrap()) * 0.5).to_le_bytes())
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
