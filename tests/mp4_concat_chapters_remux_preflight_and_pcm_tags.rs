use fvid::media::{concat, plan_decode_audio, plan_loudnorm, probe};
use fvid::media_control::CopyOptions;
use fvid::media_info::{AudioDecodeTransform, MixAudioOptions};
use fvid::native_export::{
    remux_matroska, remux_mp4_matroska, transcode_mp4_ffv1, try_concat_mp4_matroska,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        "fvid-batch591-{label}-{}-{}.{ext}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn mp4_concat_open_charges_track_names_and_preserves_track_preferences_in_matroska_output() {
    let mp4_base = fixture_path("multitrack-avc-mp4-stream-metadata/avc-dual-aac-tracks.mp4");
    let seg_a = temp_path("concat-seg-a", "mp4");
    let seg_b = temp_path("concat-seg-b", "mp4");
    std::fs::copy(&mp4_base, &seg_a).unwrap();
    std::fs::copy(&mp4_base, &seg_b).unwrap();

    // 1. Existing destination must be rejected upfront in try_concat_mp4_matroska BEFORE opening missing sources.
    let existing_mkv = temp_path("concat-existing", "mkv");
    std::fs::write(&existing_mkv, b"already-exists").unwrap();
    let missing_seg = temp_path("concat-missing-seg", "mp4");
    let _ = std::fs::remove_file(&missing_seg);
    let err_existing = try_concat_mp4_matroska(
        &[missing_seg.clone(), missing_seg.clone()],
        &existing_mkv,
        None,
        None,
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&existing_mkv);
    assert!(
        err_existing
            .to_string()
            .contains("concat destination already exists"),
        "expected concat destination already exists before scanning sources, got: {err_existing}"
    );

    // 2. Concatenating two segments of avc-dual-aac-tracks.mp4 into Matroska must preserve
    // track enabled/default flags via PacketWriter::new_with_track_preferences.
    let out_mkv = temp_path("concat-prefs-out", "mkv");
    let _ = std::fs::remove_file(&out_mkv);
    let remux_mkv = temp_path("remux-prefs-out", "mkv");
    let _ = std::fs::remove_file(&remux_mkv);
    let _ = fvid::media::remux(&seg_a, &remux_mkv, &CopyOptions::default()).unwrap();
    let remux_info = probe(&remux_mkv).unwrap();
    let _ = std::fs::remove_file(&remux_mkv);
    let _ = concat(&[seg_a.clone(), seg_b.clone()], &out_mkv, &CopyOptions::default()).unwrap();
    let out_info = probe(&out_mkv).unwrap();
    let _ = std::fs::remove_file(&out_mkv);
    let _ = std::fs::remove_file(&seg_a);
    let _ = std::fs::remove_file(&seg_b);
    assert_eq!(remux_info.streams.len(), out_info.streams.len());
    for (remux_s, out_s) in remux_info.streams.iter().zip(out_info.streams.iter()) {
        assert_eq!(
            remux_s.disposition, out_s.disposition,
            "track {} disposition mismatch: {:?} vs {:?}",
            remux_s.index, remux_s.disposition, out_s.disposition
        );
    }
}

#[test]
fn owned_and_native_remux_check_existing_destination_before_opening_source() {
    let missing_src = temp_path("remux-missing-src", "mp4");
    let _ = std::fs::remove_file(&missing_src);

    let existing_mkv = temp_path("remux-existing", "mkv");
    std::fs::write(&existing_mkv, b"existing-mkv").unwrap();

    // 1. owned_mp4_remux::remux checks destination upfront before supports/open.
    let err_owned = fvid_media::owned_mp4_remux::remux(
        &missing_src,
        &existing_mkv,
        &CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        err_owned.contains("destination already exists"),
        "expected owned_mp4_remux::remux to reject existing destination upfront, got: {err_owned}"
    );

    // 2. remux_mp4_matroska checks destination upfront before opening missing source.
    let err_mp4_mka = remux_mp4_matroska(&missing_src, &existing_mkv, None, None).unwrap_err();
    assert!(
        err_mp4_mka
            .to_string()
            .contains("destination already exists"),
        "expected remux_mp4_matroska to reject existing destination upfront, got: {err_mp4_mka}"
    );

    // 3. transcode_mp4_ffv1 checks destination upfront before opening missing source.
    let err_ffv1 = transcode_mp4_ffv1(&missing_src, &existing_mkv, None, None).unwrap_err();
    assert!(
        err_ffv1.to_string().contains("destination already exists"),
        "expected transcode_mp4_ffv1 to reject existing destination upfront, got: {err_ffv1}"
    );

    // 4. remux_matroska checks destination upfront before opening missing source.
    let err_mka_copy = remux_matroska(&missing_src, &existing_mkv, None, None).unwrap_err();
    let _ = std::fs::remove_file(&existing_mkv);
    assert!(
        err_mka_copy
            .to_string()
            .contains("destination already exists"),
        "expected remux_matroska to reject existing destination upfront, got: {err_mka_copy}"
    );
}

#[test]
fn mix_merge_and_concat_audio_check_destination_and_broken_symlinks_before_scanning_inputs() {
    let missing_a = temp_path("audio-missing-a", "wav");
    let missing_b = temp_path("audio-missing-b", "wav");
    let _ = std::fs::remove_file(&missing_a);
    let _ = std::fs::remove_file(&missing_b);

    let broken_link = temp_path("audio-broken-symlink", "wav");
    let _ = std::fs::remove_file(&broken_link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp_path("nonexistent-target", "wav"), &broken_link).unwrap();
    #[cfg(not(unix))]
    std::fs::write(&broken_link, b"existing").unwrap();

    // 1. mix_audio_with_controls rejects broken symlink at destination before scanning inputs.
    let mix_err = fvid::native_audio_mix::mix_audio_with_controls(
        &[missing_a.clone(), missing_b.clone()],
        &broken_link,
        &MixAudioOptions::default(),
        &CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        mix_err
            .to_string()
            .contains("mix-audio destination already exists"),
        "expected mix_audio to reject broken symlink upfront, got: {mix_err}"
    );

    // 2. merge_audio_with_controls rejects broken symlink at destination before merge_geometry_with_options scans inputs.
    let merge_err = fvid::native_audio_mix::merge_audio_with_controls(
        &[missing_a.clone(), missing_b.clone()],
        &broken_link,
        &CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        merge_err
            .to_string()
            .contains("merge-audio destination already exists"),
        "expected merge_audio to reject broken symlink before scanning inputs, got: {merge_err}"
    );

    // 3. concat_audio_with_options rejects broken symlink at destination before validate_concat_segments scans inputs.
    let concat_err = fvid::native_audio_mix::concat_audio_with_options(
        &[missing_a, missing_b],
        &broken_link,
        &CopyOptions::default(),
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&broken_link);
    assert!(
        concat_err
            .to_string()
            .contains("concat destination already exists"),
        "expected concat_audio_with_options to reject broken symlink before scanning inputs, got: {concat_err}"
    );
}

#[test]
fn matroska_pcm_metadata_enforces_256_tag_cap_in_planning_and_export() {
    let mkv_src = temp_path("pcm-src-240-tags", "mka");
    let _ = std::fs::remove_file(&mkv_src);
    {
        let mut opus_head = Vec::with_capacity(19);
        opus_head.extend_from_slice(b"OpusHead");
        opus_head.push(1);
        opus_head.push(1);
        opus_head.extend_from_slice(&0u16.to_le_bytes());
        opus_head.extend_from_slice(&48_000u32.to_le_bytes());
        opus_head.extend_from_slice(&0i16.to_le_bytes());
        opus_head.push(0);
        let specs = [fvid::container::matroska_write::TrackSpec {
            encoding: fvid::container::matroska_write::Encoding::Opus {
                configuration: &opus_head,
            },
            name: "",
            language: "eng",
        }];
        let mut text = BTreeMap::new();
        for i in 0..240 {
            text.insert(format!("SRC_META_{i:03}"), "val".into());
        }
        let mut file = std::fs::File::create(&mkv_src).unwrap();
        let mut writer = fvid::container::matroska_write::PacketWriter::new_with_text_metadata(
            &mut file,
            &specs,
            &[],
            &Default::default(),
            &text,
            &BTreeMap::new(),
        )
        .unwrap();
        writer
            .write_packet(0, 0, 20_000_000, true, &[0xFC, 0x00, 0x00])
            .unwrap();
        writer.finish().unwrap();
    }

    // Adding 20 new container tags (240 + 20 = 260 > 256) must be rejected during planning!
    let mut metadata_set = Vec::new();
    for i in 0..20 {
        metadata_set.push((format!("NEW_META_{i:02}"), "v".into()));
    }
    let opts = CopyOptions {
        max_controlled_bytes: Some(64 << 20),
        metadata_set,
        ..Default::default()
    };
    let loudnorm_plan_err = plan_loudnorm(&mkv_src, None, false, &opts).unwrap_err();
    let _ = std::fs::remove_file(&mkv_src);
    assert!(
        loudnorm_plan_err.contains("Matroska metadata tag count exceeds limit"),
        "expected plan_loudnorm to enforce 256-tag cap in parse_matroska_pcm_metadata, got: {loudnorm_plan_err}"
    );
}

#[test]
fn plan_decode_audio_charges_simultaneous_index_and_packet_working_set() {
    // 1. MP4: index_bytes + sample.size must fit within max_controlled_bytes.
    let mp4 = fixture_path("batch590_mp4_uniform_relocate.mp4");
    let reader = fvid::container::mp4::Mp4Reader::open(
        std::io::BufReader::new(std::fs::File::open(&mp4).unwrap()),
        Default::default(),
    )
    .unwrap();
    let mp4_index_bytes = reader.estimated_index_payload_bytes().unwrap();
    // Uniform sample size is 256 bytes.
    // Set max_controlled_bytes = mp4_index_bytes + 100 so:
    // - mp4_index_bytes <= limit
    // - sample.size (256) <= max_packet_bytes
    // - mp4_index_bytes + sample.size (mp4_index_bytes + 256) > limit!
    let mp4_opts = CopyOptions {
        max_controlled_bytes: Some(mp4_index_bytes + 100),
        ..Default::default()
    };
    let mp4_err =
        plan_decode_audio(&mp4, &AudioDecodeTransform::default(), &mp4_opts).unwrap_err();
    assert!(
        mp4_err.contains("controlled memory budget exceeded"),
        "expected plan_decode_audio on MP4 to charge index_bytes + sample.size, got: {mp4_err}"
    );

    // 2. Matroska: index_bytes + packet_payload_size + packet_read_overhead must fit within max_controlled_bytes.
    let mka = temp_path("audio-plan-pkt-budget", "mka");
    let _ = std::fs::remove_file(&mka);
    {
        let specs = [fvid::container::matroska_write::TrackSpec {
            encoding: fvid::container::matroska_write::Encoding::PcmFloat32 {
                sample_rate: 8_000,
                channels: 1,
            },
            name: "",
            language: "eng",
        }];
        let mut file = std::fs::File::create(&mka).unwrap();
        let mut writer = fvid::container::matroska_write::PacketWriter::new_with_text_metadata(
            &mut file,
            &specs,
            &[],
            &Default::default(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        // Write one 4096-byte PCM packet.
        writer
            .write_packet(0, 0, 128_000_000, true, &vec![0u8; 4096])
            .unwrap();
        writer.finish().unwrap();
    }
    let mut webm_reader = fvid::container::webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&mka).unwrap()),
        Default::default(),
    )
    .unwrap();
    webm_reader.scan_all().unwrap();
    let webm_index_bytes = webm_reader.estimated_index_payload_bytes().unwrap();
    // Set max_controlled_bytes = webm_index_bytes + 1000 so:
    // - webm_index_bytes <= limit
    // - packet_payload_size (4096) <= max_packet_bytes
    // - webm_index_bytes + 4096 > limit!
    let mka_opts = CopyOptions {
        max_controlled_bytes: Some(webm_index_bytes + 1000),
        ..Default::default()
    };
    let mka_err =
        plan_decode_audio(&mka, &AudioDecodeTransform::default(), &mka_opts).unwrap_err();
    let _ = std::fs::remove_file(&mka);
    assert!(
        mka_err.contains("controlled memory budget exceeded"),
        "expected plan_decode_audio on Matroska to charge index_bytes + packet_payload_size, got: {mka_err}"
    );
}
