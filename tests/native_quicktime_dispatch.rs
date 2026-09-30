use std::path::{Path, PathBuf};

#[test]
fn quicktime_without_file_type_atom_uses_owned_probe_and_audio_export() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/fixtures/audio/aac-native-edit.m4a");
    let original = std::fs::read(&source).unwrap();
    assert_eq!(&original[4..8], b"ftyp");
    let directory = std::env::temp_dir().join(format!("fvid-mov-dispatch-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let expected = directory.join("original.f32le");
    fvid::native_export::export_audio_pcm_selected(
        &source, &expected, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    let expected = std::fs::read(expected).unwrap();
    let baseline = fvid::native_probe::probe(&source).unwrap();
    for tag in [*b"free", *b"skip", *b"wide", *b"uuid"] {
        let input = directory.join(format!("{}.mov", String::from_utf8_lossy(&tag)));
        let mut bytes = original.clone();
        bytes[4..8].copy_from_slice(&tag);
        std::fs::write(&input, bytes).unwrap();
        let info = fvid::native_probe::try_probe_as(&input, None)
            .unwrap()
            .unwrap();
        assert_eq!(info.streams, baseline.streams);
        assert_eq!(info.duration_us, baseline.duration_us);
        assert!(fvid::native_media::is_owned_audio_source(&input).unwrap());
        let plan = fvid::native_plan::remux(&input).unwrap().unwrap();
        assert!(plan.notes.iter().any(|note| note.contains("backend: fvid")));
        let cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "remux"])
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            cli.status.success(),
            "{}",
            String::from_utf8_lossy(&cli.stderr)
        );
        let output = input.with_extension("f32le");
        fvid::native_export::export_audio_pcm_selected(
            &input, &output, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(std::fs::read(output).unwrap(), expected);
        for extension in ["mp4", "mka"] {
            let remuxed = input.with_extension(format!("cli.{extension}"));
            let cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "remux"])
                .arg(&input)
                .arg(&remuxed)
                .output()
                .unwrap();
            assert!(
                cli.status.success(),
                "{}",
                String::from_utf8_lossy(&cli.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&cli.stdout).unwrap();
            assert_eq!(result["backend"], "fvid");
            let decoded = input.with_extension(format!("cli.{extension}.f32le"));
            fvid::native_export::export_audio_pcm_selected(
                &remuxed, &decoded, None, 1.0, None, None, None, None, None,
            )
            .unwrap();
            assert_eq!(std::fs::read(decoded).unwrap(), expected);
        }
        #[cfg(feature = "media")]
        {
            let info = fvid::media::probe(&input).unwrap();
            assert_eq!(info.streams, baseline.streams);
            let output = input.with_extension("api.f32le");
            fvid::media::decode_audio(&input, &output, &Default::default()).unwrap();
            assert_eq!(std::fs::read(output).unwrap(), expected);
            for extension in ["mp4", "mka"] {
                let remuxed = input.with_extension(extension);
                let stats = fvid::media::remux(&input, &remuxed, &Default::default()).unwrap();
                assert_eq!(stats.backend, "fvid");
                let decoded = input.with_extension(format!("{extension}.f32le"));
                fvid::media::decode_audio(&remuxed, &decoded, &Default::default()).unwrap();
                assert_eq!(std::fs::read(decoded).unwrap(), expected, "{extension}");
                let published = std::fs::read(&remuxed).unwrap();
                assert!(fvid::media::remux(&input, &remuxed, &Default::default()).is_err());
                assert_eq!(std::fs::read(&remuxed).unwrap(), published);
            }
        }
    }
}

#[test]
fn recognized_truncated_quicktime_atoms_are_errors_not_unknown_formats() {
    let path = std::env::temp_dir().join(format!("fvid-mov-bad-{}", std::process::id()));
    for tag in [
        *b"moov", *b"mdat", *b"wide", *b"free", *b"skip", *b"uuid", *b"styp",
    ] {
        let mut bytes = vec![0, 0, 0, 32];
        bytes.extend_from_slice(&tag);
        std::fs::write(&path, bytes).unwrap();
        assert!(fvid::native_probe::try_probe_as(&path, None).is_err());
    }
    std::fs::remove_file(path).unwrap();
    assert!(!fvid::container::mp4::recognizes_prefix(
        b"\xff\xd8\xff\xe0JPEG"
    ));
    assert!(!fvid::container::mp4::recognizes_prefix(b"ftyp"));
}
