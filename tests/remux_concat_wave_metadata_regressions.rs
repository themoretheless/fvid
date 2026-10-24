use fvid::media::{CopyOptions, MixAudioOptions};
use std::io::Cursor;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/remux-concat-wave-metadata-precedence")
        .join(name)
}

#[test]
fn remux_concat_pcm_matroska_and_wave_metadata_enforce_set_then_delete_precedence_and_case_normalization() {
    let mp4_seg = fixture("avc-aac-segment.mp4");
    let adts_a = fixture("adts-mono-a.aac");
    let adts_b = fixture("adts-mono-b.aac");
    let wav_a = fixture("tone-info-a.wav");
    let wav_b = fixture("tone-info-b.wav");

    let temp = std::env::temp_dir().join(format!(
        "fvid-remux-concat-wave-metadata-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(temp.clone());

    // 1. `owned_mp4_remux` (`fvid_media::remux` / `fvid::media::remux` on MP4 -> Matroska)
    //    enforces set-then-delete precedence for container and stream metadata.
    let mp4_remux_opts = CopyOptions {
        streams: vec![1, 0],
        metadata_set: vec![
            ("title".into(), "Transient MP4 Remux Title".into()),
            ("artist".into(), "Retained MP4 Remux Artist".into()),
            ("custom_container".into(), "TransientCustom".into()),
            ("kept_custom".into(), "KeptCustomVal".into()),
        ],
        metadata_delete: vec!["TITLE".into(), "CUSTOM_CONTAINER".into()],
        stream_metadata_set: vec![
            (1, "title".into(), "Transient Audio Title".into()),
            (1, "language".into(), "fra".into()),
            (1, "custom_track".into(), "TransientTrackCustom".into()),
            (1, "kept_track".into(), "KeptTrackVal".into()),
            (0, "title".into(), "Kept Video Title".into()),
            (0, "language".into(), "deu".into()),
        ],
        stream_metadata_delete: vec![
            (1, "TITLE".into()),
            (1, "LANGUAGE".into()),
            (1, "CUSTOM_TRACK".into()),
        ],
        ..Default::default()
    };
    let mp4_remux_plan = fvid::media::plan_remux(&mp4_seg, &mp4_remux_opts)
        .expect("plan_remux on MP4 with set-then-delete metadata must succeed");
    assert!(
        mp4_remux_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in MP4 remux plan"
    );
    let mp4_remux_mkv = temp.join("mp4-remux.mkv");
    fvid::media::remux(&mp4_seg, &mp4_remux_mkv, &mp4_remux_opts)
        .expect("MP4 remux with set-then-delete metadata must succeed");
    let mut remux_reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&mp4_remux_mkv).unwrap()),
        Default::default(),
    )
    .unwrap();
    remux_reader.scan_all().unwrap();
    assert!(
        remux_reader.tags.title.is_empty(),
        "metadata_delete must win over metadata_set for title in MP4 remux"
    );
    assert_eq!(remux_reader.tags.artist, "Retained MP4 Remux Artist");
    assert!(
        !remux_reader.metadata.contains_key("CUSTOM_CONTAINER")
            && !remux_reader.metadata.contains_key("custom_container"),
        "metadata_delete must remove custom container tag case-insensitively in MP4 remux: {:?}",
        remux_reader.metadata
    );
    assert_eq!(
        remux_reader.metadata.get("KEPT_CUSTOM").map(String::as_str),
        Some("KeptCustomVal")
    );
    // Output track 0 corresponds to source stream 1 (audio); output track 1 corresponds to source stream 0 (video).
    assert_eq!(
        remux_reader.tracks[0].name, "",
        "stream_metadata_delete must win over stream_metadata_set for track title"
    );
    assert_eq!(
        remux_reader.tracks[0].language, "",
        "stream_metadata_delete must reset track language to und (normalized to empty on read)"
    );
    assert_eq!(
        remux_reader
            .track_languages
            .get(&remux_reader.tracks[0].number)
            .map(String::as_str),
        Some("und")
    );
    let track0_uid = remux_reader.track_uids[&remux_reader.tracks[0].number];
    let track0_tags = remux_reader
        .track_metadata
        .get(&track0_uid)
        .cloned()
        .unwrap_or_default();
    assert!(
        !track0_tags.contains_key("CUSTOM_TRACK") && !track0_tags.contains_key("custom_track"),
        "stream_metadata_delete must remove custom stream tag case-insensitively: {:?}",
        track0_tags
    );
    assert_eq!(
        track0_tags.get("KEPT_TRACK").map(String::as_str),
        Some("KeptTrackVal")
    );
    assert_eq!(remux_reader.tracks[1].name, "Kept Video Title");
    assert_eq!(remux_reader.tracks[1].language, "deu");

    // 2. Compressed MP4 concat (`src/container/mp4_concat.rs::write_with_options`)
    //    enforces set-then-delete precedence, case-insensitive deletion, and uppercase custom tags.
    let mp4_concat_mkv = temp.join("mp4-concat.mkv");
    fvid::media::concat(
        &[mp4_seg.clone(), mp4_seg.clone()],
        &mp4_concat_mkv,
        &mp4_remux_opts,
    )
    .expect("MP4 concat with set-then-delete metadata must succeed");
    let mut concat_reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&mp4_concat_mkv).unwrap()),
        Default::default(),
    )
    .unwrap();
    concat_reader.scan_all().unwrap();
    assert!(
        concat_reader.tags.title.is_empty(),
        "metadata_delete must win over metadata_set for title in MP4 concat"
    );
    assert_eq!(concat_reader.tags.artist, "Retained MP4 Remux Artist");
    assert!(
        !concat_reader.metadata.contains_key("CUSTOM_CONTAINER")
            && !concat_reader.metadata.contains_key("custom_container"),
        "metadata_delete must remove custom container tag case-insensitively in MP4 concat: {:?}",
        concat_reader.metadata
    );
    assert_eq!(
        concat_reader.metadata.get("KEPT_CUSTOM").map(String::as_str),
        Some("KeptCustomVal")
    );
    assert_eq!(concat_reader.tracks[0].name, "");
    assert_eq!(concat_reader.tracks[0].language, "");
    let concat_t0_uid = concat_reader.track_uids[&concat_reader.tracks[0].number];
    let concat_t0_tags = concat_reader
        .track_metadata
        .get(&concat_t0_uid)
        .cloned()
        .unwrap_or_default();
    assert!(
        !concat_t0_tags.contains_key("CUSTOM_TRACK")
            && !concat_t0_tags.contains_key("custom_track"),
        "stream_metadata_delete must remove custom stream tag in MP4 concat: {:?}",
        concat_t0_tags
    );
    assert_eq!(
        concat_t0_tags.get("KEPT_TRACK").map(String::as_str),
        Some("KeptTrackVal")
    );
    assert_eq!(concat_reader.tracks[1].name, "Kept Video Title");
    assert_eq!(concat_reader.tracks[1].language, "deu");

    // Also verify MP4 concat rejects forbidden video/display stream metadata keys (`rotate`, `stereo_mode`, `alpha_mode`)
    // and overly long keys/values consistently with other Matroska writers.
    let bad_mp4_concat_opts = CopyOptions {
        stream_metadata_set: vec![(0, "rotate".into(), "90".into())],
        ..Default::default()
    };
    assert!(
        fvid::media::plan_concat(&[mp4_seg.clone(), mp4_seg.clone()], &bad_mp4_concat_opts)
            .is_err(),
        "plan_concat on MP4 must reject rotate in stream_metadata_set"
    );
    let bad_mp4_concat_mkv = temp.join("bad-mp4-concat.mkv");
    assert!(
        fvid::media::concat(
            &[mp4_seg.clone(), mp4_seg.clone()],
            &bad_mp4_concat_mkv,
            &bad_mp4_concat_opts
        )
        .is_err(),
        "concat on MP4 must reject rotate in stream_metadata_set"
    );
    assert!(!bad_mp4_concat_mkv.exists());

    // 3. `owned_matroska_adts_impl::extract_adts_matroska_metadata` (`owned_adts_remux` and `owned_concat`)
    //    enforces set-then-delete precedence, case-insensitive removal, and uppercase custom tags.
    let adts_opts = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Transient ADTS Title".into()),
            ("artist".into(), "Retained ADTS Artist".into()),
            ("adts_note".into(), "TransientNote".into()),
            ("adts_kept".into(), "KeptAdtsNote".into()),
        ],
        metadata_delete: vec!["TITLE".into(), "ADTS_NOTE".into()],
        stream_metadata_set: vec![
            (0, "title".into(), "Transient ADTS Track".into()),
            (0, "language".into(), "jpn".into()),
            (0, "track_note".into(), "TransientTrackNote".into()),
            (0, "track_kept".into(), "KeptTrackNote".into()),
        ],
        stream_metadata_delete: vec![
            (0, "TITLE".into()),
            (0, "LANGUAGE".into()),
            (0, "TRACK_NOTE".into()),
        ],
        ..Default::default()
    };
    let adts_remux_mka = temp.join("adts-remux.mka");
    fvid_media::remux(&adts_a, &adts_remux_mka, &adts_opts)
        .expect("ADTS remux with set-then-delete metadata must succeed");
    let adts_concat_mka = temp.join("adts-concat.mka");
    fvid::media::concat(&[adts_a.clone(), adts_b.clone()], &adts_concat_mka, &adts_opts)
        .expect("ADTS concat with set-then-delete metadata must succeed");

    for path in [&adts_remux_mka, &adts_concat_mka] {
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(path).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert!(
            reader.tags.title.is_empty(),
            "ADTS Matroska output must delete title when in metadata_delete: {:?}",
            path
        );
        assert_eq!(reader.tags.artist, "Retained ADTS Artist");
        assert!(
            !reader.metadata.contains_key("ADTS_NOTE")
                && !reader.metadata.contains_key("adts_note"),
            "ADTS Matroska output must delete custom container tag case-insensitively: {:?}",
            reader.metadata
        );
        assert_eq!(
            reader.metadata.get("ADTS_KEPT").map(String::as_str),
            Some("KeptAdtsNote")
        );
        assert_eq!(reader.tracks[0].name, "");
        assert_eq!(reader.tracks[0].language, "");
        let uid = reader.track_uids[&reader.tracks[0].number];
        let stags = reader.track_metadata.get(&uid).cloned().unwrap_or_default();
        assert!(
            !stags.contains_key("TRACK_NOTE") && !stags.contains_key("track_note"),
            "ADTS Matroska output must delete custom stream tag case-insensitively: {:?}",
            stags
        );
        assert_eq!(
            stags.get("TRACK_KEPT").map(String::as_str),
            Some("KeptTrackNote")
        );
    }

    // 4. `src/native_export/pcm_matroska.rs::parse_matroska_pcm_metadata` (`mix_audio_with_controls`
    //    and `concat_audio_with_options` to `.mka`) enforces set-then-delete precedence and uppercase custom tags.
    let pcm_mka = temp.join("mixed-pcm.mka");
    fvid::media::mix_audio_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &pcm_mka,
        &MixAudioOptions::default(),
        &adts_opts,
    )
    .expect("mix_audio_with_controls to .mka with set-then-delete metadata must succeed");
    let mut pcm_reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&pcm_mka).unwrap()),
        Default::default(),
    )
    .unwrap();
    pcm_reader.scan_all().unwrap();
    assert!(
        pcm_reader.tags.title.is_empty(),
        "PCM Matroska output must delete title when in metadata_delete"
    );
    assert_eq!(pcm_reader.tags.artist, "Retained ADTS Artist");
    assert!(
        !pcm_reader.metadata.contains_key("ADTS_NOTE")
            && !pcm_reader.metadata.contains_key("adts_note"),
        "PCM Matroska output must delete custom container tag case-insensitively: {:?}",
        pcm_reader.metadata
    );
    assert_eq!(
        pcm_reader.metadata.get("ADTS_KEPT").map(String::as_str),
        Some("KeptAdtsNote")
    );
    assert_eq!(pcm_reader.tracks[0].name, "");
    assert_eq!(pcm_reader.tracks[0].language, "");
    let pcm_uid = pcm_reader.track_uids[&pcm_reader.tracks[0].number];
    let pcm_stags = pcm_reader
        .track_metadata
        .get(&pcm_uid)
        .cloned()
        .unwrap_or_default();
    assert!(
        !pcm_stags.contains_key("TRACK_NOTE") && !pcm_stags.contains_key("track_note"),
        "PCM Matroska output must delete custom stream tag case-insensitively: {:?}",
        pcm_stags
    );
    assert_eq!(
        pcm_stags.get("TRACK_KEPT").map(String::as_str),
        Some("KeptTrackNote")
    );

    // 5. `crates/fvid-media/src/owned_wave_metadata.rs::edit_info_chunks` enforces set-then-delete
    //    precedence across WAVE `LIST`/`INFO` exports (`decode_audio`, `mix_audio_with_controls`, `remux`).
    let wave_opts = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Transient Wave Title".into()),
            ("artist".into(), "Retained Wave Artist".into()),
            ("comment".into(), "Kept Wave Comment".into()),
        ],
        metadata_delete: vec!["TITLE".into()],
        ..Default::default()
    };
    let decoded_wav = temp.join("decoded-edited.wav");
    fvid_media::decode_audio(&wav_a, &decoded_wav, &wave_opts)
        .expect("decode_audio to .wav with set-then-delete INFO metadata must succeed");
    let probed_wav = fvid::media::probe(&decoded_wav).unwrap();
    assert!(
        !probed_wav.metadata.contains_key("title"),
        "WAVE INFO metadata_delete must win over metadata_set for title: {:?}",
        probed_wav.metadata
    );
    assert_eq!(
        probed_wav.metadata.get("artist").map(String::as_str),
        Some("Retained Wave Artist")
    );
    assert_eq!(
        probed_wav.metadata.get("comment").map(String::as_str),
        Some("Kept Wave Comment")
    );
    assert_eq!(
        probed_wav.metadata.get("QWER").map(String::as_str),
        Some("Opaque Riff Tag"),
        "untouched opaque RIFF INFO entries must be preserved"
    );

    let remuxed_wav = temp.join("remuxed-edited.wav");
    fvid_media::remux(&wav_a, &remuxed_wav, &wave_opts)
        .expect("WAVE remux with set-then-delete INFO metadata must succeed");
    let probed_remux_wav = fvid::media::probe(&remuxed_wav).unwrap();
    assert!(!probed_remux_wav.metadata.contains_key("title"));
    assert_eq!(
        probed_remux_wav.metadata.get("artist").map(String::as_str),
        Some("Retained Wave Artist")
    );
    assert_eq!(
        probed_remux_wav.metadata.get("comment").map(String::as_str),
        Some("Kept Wave Comment")
    );
    assert_eq!(
        probed_remux_wav.metadata.get("QWER").map(String::as_str),
        Some("Opaque Riff Tag")
    );
}
