use fvid::media::{self, CopyOptions};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/remux-concat-policy-parity")
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-remux-concat-policy-parity-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn matroska_remux_rejects_unsupported_policies_with_matroska_specific_diagnostics_in_plan_and_execution(
) {
    let mka = fixture_dir().join("matroska-pcm-mono-48khz.mka");
    let temp = TempDir::new("matroska-remux");

    let stream_opts = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    let out_stream = temp.0.join("stream.mka");
    let plan_err = media::plan_remux(&mka, &stream_opts).unwrap_err();
    let exec_err = media::remux(&mka, &out_stream, &stream_opts).unwrap_err();
    assert!(
        plan_err.contains("Matroska") && !plan_err.contains("RIFF"),
        "expected Matroska-specific refusal in plan_remux, got: {plan_err}"
    );
    assert_eq!(plan_err, exec_err);
    assert!(!out_stream.exists());

    let packet_cap_opts = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let out_cap = temp.0.join("cap.mka");
    let plan_cap_err = media::plan_remux(&mka, &packet_cap_opts).unwrap_err();
    let exec_cap_err = media::remux(&mka, &out_cap, &packet_cap_opts).unwrap_err();
    assert!(
        plan_cap_err.contains("Matroska") && !plan_cap_err.contains("RIFF"),
        "expected Matroska-specific refusal for max_packets in plan_remux, got: {plan_cap_err}"
    );
    assert_eq!(plan_cap_err, exec_cap_err);
    assert!(!out_cap.exists());

    let zero_pkt_opts = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let out_zero = temp.0.join("zero.mka");
    let plan_zero_err = media::plan_remux(&mka, &zero_pkt_opts).unwrap_err();
    let exec_zero_err = media::remux(&mka, &out_zero, &zero_pkt_opts).unwrap_err();
    assert!(
        plan_zero_err.contains("packet byte limit must be positive"),
        "expected positive packet byte limit error, got: {plan_zero_err}"
    );
    assert_eq!(plan_zero_err, exec_zero_err);
    assert!(!out_zero.exists());
}

#[test]
fn mp4_remux_enforces_packet_budget_and_option_parity_between_plan_and_execution() {
    let mp4 = fixture_dir().join("mp4-aac-mono-48khz.m4a");
    let temp = TempDir::new("mp4-remux");

    // 4-byte AAC packets in mp4-aac-mono-48khz.m4a; max_packet_bytes = 3 must fail in both plan and execution.
    let tight_budget = CopyOptions {
        max_packet_bytes: 3,
        ..Default::default()
    };
    let out_mp4 = temp.0.join("tight.m4a");
    let out_mka = temp.0.join("tight.mka");
    let plan_err = media::plan_remux(&mp4, &tight_budget).unwrap_err();
    let exec_mp4_err = media::remux(&mp4, &out_mp4, &tight_budget).unwrap_err();
    let exec_mka_err = media::remux(&mp4, &out_mka, &tight_budget).unwrap_err();
    assert!(
        plan_err.contains("MP4 packet exceeds budget")
            || plan_err.contains("exceeds packet byte limit"),
        "expected MP4 packet budget error in plan_remux, got: {plan_err}"
    );
    assert_eq!(plan_err, exec_mp4_err);
    assert_eq!(plan_err, exec_mka_err);
    assert!(!out_mp4.exists());
    assert!(!out_mka.exists());

    // Sufficient packet budget (max_packet_bytes = 4) succeeds in both plan_remux and remux.
    let ok_budget = CopyOptions {
        max_packet_bytes: 4,
        ..Default::default()
    };
    let ok_plan = media::plan_remux(&mp4, &ok_budget).unwrap();
    assert_eq!(ok_plan.command, "remux");
    let out_ok_mka = temp.0.join("ok.mka");
    let stats_mka = media::remux(&mp4, &out_ok_mka, &ok_budget).unwrap();
    assert_eq!(stats_mka.packets, 2);
    assert!(out_ok_mka.is_file());

    // Invalid stream selection on MP4 remux returns an MP4-specific error, never "file is not a RIFF/WAVE stream".
    let bad_stream = CopyOptions {
        streams: vec![9],
        ..Default::default()
    };
    let out_bad_stream = temp.0.join("bad-stream.mka");
    let plan_stream_err = media::plan_remux(&mp4, &bad_stream).unwrap_err();
    let exec_stream_err = media::remux(&mp4, &out_bad_stream, &bad_stream).unwrap_err();
    assert!(
        !plan_stream_err.contains("RIFF"),
        "expected MP4-specific error in plan_remux, got: {plan_stream_err}"
    );
    assert_eq!(plan_stream_err, exec_stream_err);
    assert!(!out_bad_stream.exists());
}

#[test]
fn non_wave_concat_with_invalid_policy_reports_container_error_not_riff_wave() {
    let mka = fixture_dir().join("matroska-pcm-mono-48khz.mka");
    let sources = vec![mka.clone(), mka];
    let temp = TempDir::new("concat-policy");
    let out_mka = temp.0.join("joined.mka");

    let bad_stream = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    let plan_err = media::plan_concat(&sources, &bad_stream).unwrap_err();
    let exec_err = media::concat(&sources, &out_mka, &bad_stream).unwrap_err();
    let fvid_plan_err = fvid_media::plan_concat(&sources, &bad_stream).unwrap_err();
    let fvid_exec_err = fvid_media::concat(&sources, &out_mka, &bad_stream).unwrap_err();
    assert!(
        !plan_err.contains("RIFF") && !plan_err.to_ascii_lowercase().contains("wave"),
        "expected non-WAVE concat refusal for Matroska inputs with streams=[1], got: {plan_err}"
    );
    assert!(
        !fvid_plan_err.contains("RIFF") && !fvid_plan_err.to_ascii_lowercase().contains("wave"),
        "expected non-WAVE fvid_media::plan_concat refusal for Matroska inputs, got: {fvid_plan_err}"
    );
    assert_eq!(plan_err, exec_err);
    assert_eq!(fvid_plan_err, fvid_exec_err);
    assert!(!out_mka.exists());

    let controlled_budget = CopyOptions {
        max_controlled_bytes: Some(1024),
        ..Default::default()
    };
    let out_budget = temp.0.join("budget.mka");
    let plan_budget_err = media::plan_concat(&sources, &controlled_budget).unwrap_err();
    let exec_budget_err = media::concat(&sources, &out_budget, &controlled_budget).unwrap_err();
    assert!(
        plan_budget_err.contains("controlled memory budget exceeded"),
        "expected controlled memory budget error in plan_concat, got: {plan_budget_err}"
    );
    assert_eq!(plan_budget_err, exec_budget_err);
    assert!(!out_budget.exists());

    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let out_zero = temp.0.join("zero.mka");
    let plan_zero_err = media::plan_concat(&sources, &zero_packets).unwrap_err();
    let exec_zero_err = media::concat(&sources, &out_zero, &zero_packets).unwrap_err();
    assert!(
        plan_zero_err.contains("packet work limit requires at least one packet"),
        "expected packet work limit refusal in plan_concat, got: {plan_zero_err}"
    );
    assert_eq!(plan_zero_err, exec_zero_err);
    assert!(!out_zero.exists());
}
