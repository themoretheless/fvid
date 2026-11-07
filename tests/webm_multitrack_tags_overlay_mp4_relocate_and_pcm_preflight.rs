use fvid::media::{
    decode_audio, plan_remux, plan_transcode_lossless, probe, remux, transcode_lossless,
};
use fvid::media_control::{CopyOptions, ProgressEvent, ProgressHook};
use fvid::media_info::{LosslessTransform, OverlaySpec};
use fvid::native_export::{export_audio_pcm_transformed_with_controls, overlay_video_transformed};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join(name)
}

fn temp_path(label: &str, ext: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch590-{label}-{}-{}.{ext}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn webm_multitrack_lossless_qualify_enforces_256_tag_cap_and_audio_title_language_budget() {
    let mkv_path = temp_path("src-multi-tags", "mkv");
    let _ = std::fs::remove_file(&mkv_path);
    {
        let mut frame = fvid_media::owned_frame::GeometryFrame {
            width: 4,
            height: 4,
            subsampling: Some([2, 2]),
            data: vec![0x60; 24],
        };
        let ffv1_pkt = fvid_media::owned_ffv1_encoder::encode(&mut frame, 8).unwrap();
        let mut opus_head = Vec::with_capacity(19);
        opus_head.extend_from_slice(b"OpusHead");
        opus_head.push(1);
        opus_head.push(1);
        opus_head.extend_from_slice(&0u16.to_le_bytes());
        opus_head.extend_from_slice(&48_000u32.to_le_bytes());
        opus_head.extend_from_slice(&0i16.to_le_bytes());
        opus_head.push(0);
        let opus_pkt = vec![0xFC, 0x00, 0x00];
        let specs = [
            fvid_media::owned_matroska::TrackSpec {
                encoding: fvid_media::owned_matroska::Encoding::Ffv1V1 {
                    width: 4,
                    height: 4,
                },
                name: "",
                language: "eng",
            },
            fvid_media::owned_matroska::TrackSpec {
                encoding: fvid_media::owned_matroska::Encoding::Opus {
                    configuration: &opus_head,
                },
                name: "",
                language: "eng",
            },
        ];
        let track_options = [
            fvid_media::owned_matroska::TrackOptions {
                default_duration_ns: 40_000_000,
                ..Default::default()
            },
            fvid_media::owned_matroska::TrackOptions {
                default_duration_ns: 20_000_000,
                ..Default::default()
            },
        ];
        let file_meta = fvid_media::owned_matroska::FileMetadata::default();
        let mut scoped: BTreeMap<usize, BTreeMap<String, String>> = BTreeMap::new();
        let mut audio_tags = BTreeMap::new();
        for i in 0..240 {
            audio_tags.insert(format!("SRC_TAG_{i:03}"), "v".into());
        }
        scoped.insert(1, audio_tags);
        let mut file = std::fs::File::create(&mkv_path).unwrap();
        let mut writer = fvid_media::owned_matroska::PacketWriter::new_with_text_metadata(
            &mut file,
            &specs,
            &track_options,
            &file_meta,
            &BTreeMap::new(),
            &scoped,
        )
        .unwrap();
        writer
            .write_packet(0, 0, 40_000_000, true, &ffv1_pkt)
            .unwrap();
        writer
            .write_packet(1, 0, 20_000_000, true, &opus_pkt)
            .unwrap();
        writer.finish().unwrap();
    }

    // 1. Adding 20 new tags to audio track 1 brings total tags on track 1 to 260 (> 256).
    // Both plan_transcode_lossless and transcode_lossless must reject upfront in qualify.
    let mut stream_metadata_set = Vec::new();
    for i in 0..20 {
        stream_metadata_set.push((1usize, format!("ADDED_TAG_{i:02}"), "x".into()));
    }
    let opts_overflow = CopyOptions {
        stream_metadata_set,
        ..Default::default()
    };
    let transform = LosslessTransform::default();
    let plan_err = plan_transcode_lossless(&mkv_path, &transform, &opts_overflow, None).unwrap_err();
    assert!(
        plan_err.contains("Matroska text tag count exceeds limit"),
        "expected 256-tag cap refusal in plan, got: {plan_err}"
    );
    let out_mkv = temp_path("out-multi-tags", "mkv");
    let _ = std::fs::remove_file(&out_mkv);
    let export_err = transcode_lossless(&mkv_path, &out_mkv, transform.clone(), &opts_overflow).unwrap_err();
    assert!(
        export_err.contains("Matroska text tag count exceeds limit"),
        "expected 256-tag cap refusal in export, got: {export_err}"
    );

    // 2. Setting a 1000-byte "title" on audio track 1 must be charged against meta_working_bytes (~8_000 bytes).
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&mkv_path).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let index_bytes = reader.estimated_index_payload_bytes().unwrap();
    // 240 existing tags of ("1:SRC_TAG_NNN", "v") cost 240 * (13 + 1 + 32) * 8 = 88_320 bytes + 48 = 88_368 bytes.
    // Adding a 1000-byte "title" on audio track 1 adds 1000 * 8 = 8_000 bytes -> ~96_368 bytes.
    // With max_controlled_bytes = index_bytes + 90_000:
    // - Without the title mutation, index_and_meta_bytes = index_bytes + 88_368 <= limit.
    // - With the 1000-byte audio title mutation, index_and_meta_bytes = index_bytes + 96_368 > limit!
    let opts_title_budget = CopyOptions {
        max_controlled_bytes: Some(index_bytes + 90_000),
        stream_metadata_set: vec![(1, "title".into(), "T".repeat(1_000))],
        ..Default::default()
    };
    let title_plan_err =
        plan_transcode_lossless(&mkv_path, &transform, &opts_title_budget, None).unwrap_err();
    assert!(
        title_plan_err.contains("controlled memory budget exceeded"),
        "expected audio track title override to be charged in qualify, got: {title_plan_err}"
    );
    let _ = std::fs::remove_file(&mkv_path);
}

#[test]
fn native_overlay_checks_destination_before_main_and_mp4_lossless_preserves_encoder_tag() {
    // 1. Existing destination must be rejected BEFORE opening/scanning a missing main input.
    let existing = temp_path("overlay-existing", "mkv");
    std::fs::write(&existing, b"existing-overlay").unwrap();
    let missing_main = temp_path("overlay-missing-main", "mp4");
    let _ = std::fs::remove_file(&missing_main);
    let err = overlay_video_transformed(
        &missing_main,
        &missing_main,
        &existing,
        0,
        0,
        None,
        None,
        &Default::default(),
        &Default::default(),
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&existing);
    assert!(
        err.to_string().contains("overlay destination already exists"),
        "expected overlay destination check before opening main input, got: {err}"
    );

    // 2. Native MP4 lossless export (`write_mp4_selected`) must preserve MP4 ©too encoder tag.
    let mp4_base = fixture_path("multitrack-avc-mp4-stream-metadata/avc-dual-aac-tracks.mp4");
    let mut mp4_bytes = std::fs::read(&mp4_base).unwrap();
    let enc_str = b"FVidCustomEncoder";
    let mut data_box = Vec::new();
    data_box.extend_from_slice(&((16 + enc_str.len()) as u32).to_be_bytes());
    data_box.extend_from_slice(b"data");
    data_box.extend_from_slice(&1u32.to_be_bytes());
    data_box.extend_from_slice(&0u32.to_be_bytes());
    data_box.extend_from_slice(enc_str);
    let mut ctoo_box = Vec::new();
    ctoo_box.extend_from_slice(&((8 + data_box.len()) as u32).to_be_bytes());
    ctoo_box.extend_from_slice(b"\xa9too");
    ctoo_box.extend_from_slice(&data_box);
    let mut ilst_box = Vec::new();
    ilst_box.extend_from_slice(&((8 + ctoo_box.len()) as u32).to_be_bytes());
    ilst_box.extend_from_slice(b"ilst");
    ilst_box.extend_from_slice(&ctoo_box);
    let mut hdlr_box = Vec::new();
    hdlr_box.extend_from_slice(&33u32.to_be_bytes());
    hdlr_box.extend_from_slice(b"hdlr");
    hdlr_box.extend_from_slice(&0u32.to_be_bytes());
    hdlr_box.extend_from_slice(&0u32.to_be_bytes());
    hdlr_box.extend_from_slice(b"mdirappl");
    hdlr_box.extend_from_slice(&[0u8; 9]);
    let mut meta_box = Vec::new();
    meta_box.extend_from_slice(&((12 + hdlr_box.len() + ilst_box.len()) as u32).to_be_bytes());
    meta_box.extend_from_slice(b"meta");
    meta_box.extend_from_slice(&0u32.to_be_bytes());
    meta_box.extend_from_slice(&hdlr_box);
    meta_box.extend_from_slice(&ilst_box);
    let mut udta_box = Vec::new();
    udta_box.extend_from_slice(&((8 + meta_box.len()) as u32).to_be_bytes());
    udta_box.extend_from_slice(b"udta");
    udta_box.extend_from_slice(&meta_box);
    let moov_pos = mp4_bytes
        .windows(4)
        .position(|w| w == b"moov")
        .unwrap()
        - 4;
    let old_moov_len =
        u32::from_be_bytes(mp4_bytes[moov_pos..moov_pos + 4].try_into().unwrap()) as usize;
    let new_moov_len = (old_moov_len + udta_box.len()) as u32;
    mp4_bytes[moov_pos..moov_pos + 4].copy_from_slice(&new_moov_len.to_be_bytes());
    mp4_bytes.extend_from_slice(&udta_box);

    let mp4_src = temp_path("mp4-with-encoder", "mp4");
    std::fs::write(&mp4_src, &mp4_bytes).unwrap();
    let out_mkv = temp_path("mp4-lossless-encoder", "mkv");
    let _ = std::fs::remove_file(&out_mkv);
    {
        let mut out_file = std::fs::File::create(&out_mkv).unwrap();
        let (stats, _) =
            fvid::native_lossless::write_mp4(&mp4_src, &mut out_file, None, None).unwrap();
        assert!(stats.video_frames > 0);
    }
    let _ = std::fs::remove_file(&mp4_src);
    let info = probe(&out_mkv).unwrap();
    let _ = std::fs::remove_file(&out_mkv);
    assert_eq!(
        info.metadata
            .get("ENCODER")
            .or_else(|| info.metadata.get("encoder"))
            .map(String::as_str),
        Some("FVidCustomEncoder"),
        "expected MP4 ©too encoder tag to be preserved in native MP4 lossless output, got: {:?}",
        info.metadata
    );
}

#[test]
fn mp4_fast_start_relocate_charges_index_plus_sample_size_in_plan_and_remux() {
    let mp4 = fixture_path("batch590_mp4_uniform_relocate.mp4");
    let reader = fvid::container::mp4::Mp4Reader::open(
        std::io::BufReader::new(std::fs::File::open(&mp4).unwrap()),
        Default::default(),
    )
    .unwrap();
    let index_bytes = reader.estimated_index_payload_bytes().unwrap();
    // Uniform sample size is 256 bytes.
    // Set max_controlled_bytes = index_bytes + 100 so:
    // - index_bytes <= limit
    // - sample.size (256) <= limit
    // - index_bytes + sample.size (index_bytes + 256) > limit!
    let opts = CopyOptions {
        max_controlled_bytes: Some(index_bytes + 100),
        ..Default::default()
    };
    let plan_err = plan_remux(&mp4, &opts).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "expected plan_remux to charge index_bytes + sample.size, got: {plan_err}"
    );
    let out_mp4 = temp_path("relocate-budget", "mp4");
    let _ = std::fs::remove_file(&out_mp4);
    let remux_err = remux(&mp4, &out_mp4, &opts).unwrap_err();
    assert!(
        remux_err.contains("controlled memory budget exceeded"),
        "expected remux to charge index_bytes + sample.size, got: {remux_err}"
    );
}

#[test]
fn y4m_lossless_overlay_and_ffv1_encode_align_plan_and_execution_budget_diagnostics() {
    let y4m = fixture_path("batch589_valid_25fps.y4m");
    let ov_y4m = fixture_path("batch589_valid_25fps.y4m");

    // 1. Overlay frame_len (6 bytes) + meta_working_bytes:
    // Set metadata_set with 1000-byte tag -> meta_working_bytes = (10 + 1000 + 32 + 6) * 8 = 8_384 bytes.
    // With max_controlled_bytes = 8_386:
    // - meta_working_bytes (8_384) <= 8_386
    // - overlay frame_len (6) <= 8_386
    // - overlay frame_len + meta_working_bytes (8_390) > 8_386!
    let transform_ov = LosslessTransform {
        overlay: Some(OverlaySpec {
            path: ov_y4m,
            x: 0,
            y: 0,
        }),
        ..Default::default()
    };
    let opts_ov = CopyOptions {
        max_controlled_bytes: Some(8_386),
        metadata_set: vec![("CUSTOM_TAG".into(), "V".repeat(1_000))],
        ..Default::default()
    };
    let plan_ov_err = plan_transcode_lossless(&y4m, &transform_ov, &opts_ov, None).unwrap_err();
    let out_mkv = temp_path("y4m-ov-budget", "mkv");
    let _ = std::fs::remove_file(&out_mkv);
    let exec_ov_err = transcode_lossless(&y4m, &out_mkv, transform_ov, &opts_ov).unwrap_err();
    assert_eq!(plan_ov_err, exec_ov_err);
    assert!(plan_ov_err.contains("Y4M frame byte length exceeds controlled memory budget"));

    // 2. When frame_working_bytes <= limit (48 + 18 = 66 <= 75), but frame_working_bytes + encoded FFV1 packet > limit,
    // both plan_transcode_lossless and transcode_lossless must report "encoded FFV1 packet exceeds controlled memory budget".
    let opts_pkt = CopyOptions {
        max_controlled_bytes: Some(75),
        ..Default::default()
    };
    let plan_pkt_err =
        plan_transcode_lossless(&y4m, &LosslessTransform::default(), &opts_pkt, None).unwrap_err();
    let exec_pkt_err =
        transcode_lossless(&y4m, &out_mkv, LosslessTransform::default(), &opts_pkt).unwrap_err();
    assert_eq!(plan_pkt_err, exec_pkt_err);
    assert!(
        exec_pkt_err.contains("encoded FFV1 packet exceeds controlled memory budget"),
        "unexpected error: {exec_pkt_err}"
    );
}

#[test]
fn native_audio_pcm_export_checks_existing_output_and_defers_zero_progress_until_preflight() {
    let wav = fixture_path("batch589_pcm16_8k.wav");
    let existing = temp_path("pcm-existing", "wav");
    std::fs::write(&existing, b"existing-pcm").unwrap();

    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let hook_events = Arc::clone(&events);
    let hook = ProgressHook::new(move |ev| {
        hook_events.lock().unwrap().push(ev);
    });

    // 1. Existing output must fail upfront with "output already exists" and 0 progress events.
    let opts_existing = CopyOptions {
        progress: Some(hook.clone()),
        ..Default::default()
    };
    let err_existing = export_audio_pcm_transformed_with_controls(
        &wav,
        &existing,
        None,
        1.0,
        None,
        None,
        &opts_existing,
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&existing);
    assert!(
        err_existing.to_string().contains("output already exists"),
        "expected output already exists, got: {err_existing}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "existing output refusal must not emit a 0-packet progress event"
    );

    // 2. Invalid sample rate or tight controlled memory budget must fail before emitting a 0-packet progress event.
    let out_wav = temp_path("pcm-budget-fail", "wav");
    let _ = std::fs::remove_file(&out_wav);
    let err_rate = export_audio_pcm_transformed_with_controls(
        &wav,
        &out_wav,
        None,
        1.0,
        None,
        Some(100),
        &opts_existing,
    )
    .unwrap_err();
    assert!(err_rate.to_string().contains("sample rate must be within"));
    assert!(events.lock().unwrap().is_empty());

    let opts_tight = CopyOptions {
        max_controlled_bytes: Some(1024),
        progress: Some(hook),
        ..Default::default()
    };
    let err_tight = decode_audio(&wav, &out_wav, &opts_tight).unwrap_err();
    assert!(
        err_tight.contains("controlled memory budget exceeded"),
        "expected controlled memory budget exceeded, got: {err_tight}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "failed audio decode preflight must not emit a 0-packet progress event"
    );
}
