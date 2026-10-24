use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-selection")
}

fn as_f32(bytes: &[u8]) -> Vec<f32> {
    assert_eq!(bytes.len() % 4, 0);
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn remux_to_matroska(mp4_bytes: &[u8]) -> Vec<u8> {
    let mut reader =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(mp4_bytes), Default::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    fvid::container::mp4_matroska::write_selected(
        &mut reader,
        &mut output,
        None,
        None,
        None,
        &[0, 1],
    )
    .unwrap();
    output.into_inner()
}

#[test]
fn existing_audiovisual_fixtures_resolve_enabled_audio_in_native_media_and_plan() {
    for rel in [
        "tests/fixtures/playback-errors/mp4-enabled-tracks/audiovisual.mp4",
        "tests/fixtures/playback-errors/mp4-matroska-usability/audiovisual.mp4",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        assert!(fvid::native_media::is_owned_audio_source(&path).unwrap(), "{rel}");
        let info = fvid::native_media::audio_source_info_selected(&path, None).unwrap();
        assert_eq!(info.stream_index, 2, "{rel}");
        let plan = fvid::native_plan::decode_audio(&path, &Default::default()).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].index, 2, "{rel}");
    }
}

#[test]
fn automatic_audio_selection_and_inband_ps_export_match_across_mp4_and_matroska() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();

    for control in manifest["controls"].as_array().unwrap() {
        let file = control["file"].as_str().unwrap();
        let path = root().join(file);
        let data = fs::read(&path).unwrap();
        let codec = control["codec"].as_str().unwrap();
        let rate = control["sample_rate"].as_u64().unwrap() as u32;
        let channels = control["channels"].as_u64().unwrap() as u16;
        let frames = control["sample_frames"].as_u64().unwrap();
        let expected_idx = control["selected_stream_index"].as_u64().unwrap() as usize;
        let expected_pcm = as_f32(&fs::read(root().join(control["expected_pcm"].as_str().unwrap())).unwrap());

        assert!(fvid::native_media::is_owned_audio_source(&path).unwrap());
        let info = fvid::native_media::audio_source_info_selected(&path, None).unwrap();
        assert_eq!(
            (info.stream_index, info.sample_rate, info.channels, info.codec),
            (expected_idx, rate, channels, codec)
        );
        let plan = fvid::native_plan::decode_audio(&path, &Default::default()).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].index, expected_idx);

        let wav_dest = std::env::temp_dir().join(format!(
            "fvid-batch-181-{}-{}.f32le",
            file,
            std::process::id()
        ));
        let _ = fs::remove_file(&wav_dest);
        let export_stats = fvid::native_export::export_audio_pcm_selected(
            &path, &wav_dest, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(
            (export_stats.sample_rate, export_stats.channels, export_stats.sample_frames),
            (rate, channels, frames)
        );
        let exported = as_f32(&fs::read(&wav_dest).unwrap());
        fs::remove_file(&wav_dest).unwrap();
        assert_eq!(exported.len(), expected_pcm.len());
        for (a, e) in exported.iter().zip(&expected_pcm) {
            assert!((a - e).abs() <= 1e-6, "{file}: {a} != {e}");
        }

        if codec == "aac" {
            let aac_info = fvid::native_media::aac_source_info(&path).unwrap();
            assert_eq!(
                (aac_info.stream_index, aac_info.sample_rate, aac_info.channels),
                (expected_idx, rate, channels)
            );
            let mut decoded = Vec::new();
            let aac_stats = fvid::native_media::decode_mp4_aac_pcm(&data, &mut decoded).unwrap();
            assert_eq!(
                (aac_stats.sample_rate, aac_stats.channels, aac_stats.sample_frames),
                (rate, channels, frames)
            );
            let actual = as_f32(&decoded);
            for (a, e) in actual.iter().zip(&expected_pcm) {
                assert!((a - e).abs() <= 1e-6, "{file}: {a} != {e}");
            }

            // Remux the dual-audio-only MP4 to Matroska and verify Matroska automatic track selection:
            let audio_only_data = fs::read(root().join("disabled-first-audio-only.mp4")).unwrap();
            let mkv_bytes = remux_to_matroska(&audio_only_data);
            let mkv_path = std::env::temp_dir().join(format!(
                "fvid-batch-181-remux-{}-{}.mka",
                file,
                std::process::id()
            ));
            fs::write(&mkv_path, &mkv_bytes).unwrap();
            let mkv_info = fvid::native_media::audio_source_info_selected(&mkv_path, None).unwrap();
            assert_eq!(
                (mkv_info.stream_index, mkv_info.sample_rate, mkv_info.channels, mkv_info.codec),
                (1, rate, channels, "aac")
            );
            let mkv_aac_info = fvid::native_media::aac_source_info(&mkv_path).unwrap();
            assert_eq!(
                (mkv_aac_info.stream_index, mkv_aac_info.sample_rate, mkv_aac_info.channels),
                (1, rate, channels)
            );
            let mut mkv_decoded = Vec::new();
            let mkv_stats = fvid::native_media::decode_matroska_aac_pcm_interval(
                &mkv_bytes,
                &mut mkv_decoded,
                None,
            )
            .unwrap();
            assert_eq!(
                (mkv_stats.sample_rate, mkv_stats.channels, mkv_stats.sample_frames),
                (rate, channels, frames)
            );
            let mkv_actual = as_f32(&mkv_decoded);
            for (a, e) in mkv_actual.iter().zip(&expected_pcm) {
                assert!((a - e).abs() <= 1e-6, "{file} mkv: {a} != {e}");
            }
            fs::remove_file(&mkv_path).unwrap();
        }

        // Explicit selection of the disabled first audio track (stream index 1) still works:
        let explicit_info =
            fvid::native_media::audio_source_info_selected(&path, Some(1)).unwrap();
        assert_eq!(explicit_info.stream_index, 1);
    }

    // Ambiguous dual-enabled tracks (flags=[3, 3]) still refuse automatic selection:
    for case in manifest["ambiguous"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let expected_err = case["expected_error"].as_str().unwrap();
        let path = root().join(file);
        let err = fvid::native_media::audio_source_info_selected(&path, None).err().unwrap();
        assert!(err.to_string().contains(expected_err), "{file}: {err}");
        let err = fvid::native_media::aac_source_info(&path).err().unwrap();
        assert!(err.to_string().contains(expected_err), "{file}: {err}");
    }

    // Verify in-band PS export through native_export::export_audio_pcm_selected produces stereo (2 ch, 9600 frames):
    let ps_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/he-aac-ps-inband-export-lc-960-synthetic.mp4");
    let wav_dest = std::env::temp_dir().join(format!(
        "fvid-batch-181-ps-valid-{}.wav",
        std::process::id()
    ));
    let _ = fs::remove_file(&wav_dest);
    let stats = fvid::native_export::export_audio_pcm_selected(
        &ps_source, &wav_dest, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (48000, 2, 9600)
    );
    let mut wave = fs::File::open(&wav_dest).unwrap();
    let wave_info = fvid_media::owned_wave_inspect::inspect(&mut wave, None).unwrap();
    assert_eq!(
        (wave_info.sample_rate, wave_info.channels, wave_info.sample_frames),
        (48000, 2, 9600)
    );
    fs::remove_file(&wav_dest).unwrap();
}
