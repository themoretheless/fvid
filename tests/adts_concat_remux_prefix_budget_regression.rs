use fvid::media;
use fvid::media_control::CopyOptions;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "fvid-batch271-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/adts-concat-remux-prefix-budget")
}

#[test]
fn adts_concat_and_remux_scope_packet_budget_to_active_prefix_and_report_negotiated_sbr_rate() {
    let dir = fixture_dir();
    let seg_a = dir.join("seg-a-small.aac");
    let seg_b = dir.join("seg-b-large.aac");
    let seg_c = dir.join("seg-c-invalid-rate.aac");
    let sbr_aac = dir.join("sbr-24k-to-48k.aac");
    let tmp = TempDir::new("adts-concat-remux");

    let sources = vec![seg_a.clone(), seg_b.clone()];

    // 1. With max_packets = Some(2) and max_packet_bytes = 16, only seg_a (two 8-byte packets)
    // is within the active packet prefix. seg_b (one 32+ byte packet) is never read.
    // All four ADTS concat entrypoints (media::plan_concat, fvid_media::plan_concat,
    // media::concat to .m4a, and media::concat / fvid_media::concat to .mka) must succeed.
    let prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(2),
        ..Default::default()
    };

    let native_plan = media::plan_concat(&sources, &prefix_opts)
        .expect("media::plan_concat should admit 2-packet ADTS prefix within 16-byte budget");
    assert_eq!(native_plan.command, "concat");
    assert!(
        native_plan.steps[0]
            .detail
            .contains("append 2 AAC packets into one MP4 or Matroska track without decoding"),
        "unexpected native plan step: {}",
        native_plan.steps[0].detail
    );

    let owned_plan = fvid_media::plan_concat(&sources, &prefix_opts)
        .expect("fvid_media::plan_concat should admit 2-packet ADTS prefix within 16-byte budget");
    assert_eq!(owned_plan.command, "concat");
    assert!(
        owned_plan.steps[0]
            .detail
            .contains("copy 2 unchanged AAC packets, 16 payload bytes across 2 segments; 48000 Hz, 1 channels"),
        "unexpected owned plan step: {}",
        owned_plan.steps[0].detail
    );

    let out_m4a = tmp.path().join("concat-prefix.m4a");
    let stats_m4a = media::concat(&sources, &out_m4a, &prefix_opts)
        .expect("media::concat to .m4a should succeed for 2-packet ADTS prefix");
    assert_eq!(stats_m4a.packets, 2);
    assert_eq!(stats_m4a.payload_bytes, 16);
    assert_eq!(stats_m4a.segments, 2);

    let out_mka = tmp.path().join("concat-prefix.mka");
    let stats_mka = media::concat(&sources, &out_mka, &prefix_opts)
        .expect("media::concat to .mka should succeed for 2-packet ADTS prefix");
    assert_eq!(stats_mka.packets, 2);
    assert_eq!(stats_mka.payload_bytes, 16);
    assert_eq!(stats_mka.segments, 2);

    let out_owned_mka = tmp.path().join("concat-owned-prefix.mka");
    let stats_owned_mka = fvid_media::concat(&sources, &out_owned_mka, &prefix_opts)
        .expect("fvid_media::concat to .mka should succeed for 2-packet ADTS prefix");
    assert_eq!(stats_owned_mka.packets, 2);
    assert_eq!(stats_owned_mka.payload_bytes, 16);

    // 2. Even when max_packets = Some(1) stops in seg_a, structural configuration mismatch
    // in seg_c (44.1 kHz vs 48 kHz) must still be rejected by both plan_concat and concat:
    let mismatch_sources = vec![seg_a.clone(), seg_c];
    let one_pkt_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(1),
        ..Default::default()
    };
    let err_mismatch_native = media::plan_concat(&mismatch_sources, &one_pkt_opts).unwrap_err();
    assert!(
        err_mismatch_native.contains("ADTS concat requires identical AAC configurations"),
        "unexpected mismatch error: {err_mismatch_native}"
    );
    let err_mismatch_owned = fvid_media::plan_concat(&mismatch_sources, &one_pkt_opts).unwrap_err();
    assert!(
        err_mismatch_owned.contains("ADTS concat requires identical AAC configurations"),
        "unexpected owned mismatch error: {err_mismatch_owned}"
    );
    let out_mismatch_m4a = tmp.path().join("mismatch.m4a");
    let err_mismatch_exec =
        media::concat(&mismatch_sources, &out_mismatch_m4a, &one_pkt_opts).unwrap_err();
    assert!(
        err_mismatch_exec.contains("ADTS concat requires identical AAC configurations"),
        "unexpected mismatch exec error: {err_mismatch_exec}"
    );
    assert!(!out_mismatch_m4a.exists());

    // 3. When max_packets = Some(3) reaches seg_b (whose packet exceeds 16 bytes),
    // both plan_concat and concat must reject with "ADTS packet exceeds budget":
    let exceed_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(3),
        ..Default::default()
    };
    let err_exceed_native = media::plan_concat(&sources, &exceed_opts).unwrap_err();
    assert!(
        err_exceed_native.contains("ADTS packet exceeds budget"),
        "unexpected exceed error: {err_exceed_native}"
    );
    let err_exceed_owned = fvid_media::plan_concat(&sources, &exceed_opts).unwrap_err();
    assert!(
        err_exceed_owned.contains("ADTS packet exceeds budget"),
        "unexpected owned exceed error: {err_exceed_owned}"
    );
    let out_exceed_m4a = tmp.path().join("exceed.m4a");
    let err_exceed_exec = media::concat(&sources, &out_exceed_m4a, &exceed_opts).unwrap_err();
    assert!(
        err_exceed_exec.contains("ADTS packet exceeds budget"),
        "unexpected exceed exec error: {err_exceed_exec}"
    );
    assert!(!out_exceed_m4a.exists());

    // 4. Implicit SBR rate negotiation and budget notes in fvid_media::plan_remux and fvid_media::plan_concat:
    // sbr-24k-to-48k.aac has a 24 kHz ADTS header with implicit SBR in packet 0 (negotiating 48000 Hz)
    // and a >24-byte packet 1. With max_packets = Some(1) and max_packet_bytes = 24:
    let sbr_opts = CopyOptions {
        max_packet_bytes: 24,
        max_packets: Some(1),
        ..Default::default()
    };
    let sbr_remux_plan = fvid_media::plan_remux(&sbr_aac, &sbr_opts)
        .expect("fvid_media::plan_remux should succeed for 1-packet SBR prefix");
    assert!(
        sbr_remux_plan.steps[0].detail.contains("48000 Hz, 1 channels"),
        "expected negotiated 48000 Hz in plan_remux step, got: {}",
        sbr_remux_plan.steps[0].detail
    );
    assert!(
        sbr_remux_plan
            .notes
            .iter()
            .any(|n| n.contains("encoded packet limit: 24 bytes; packet work limit: 1 packets")),
        "expected budget note in fvid_media::plan_remux, got: {:?}",
        sbr_remux_plan.notes
    );

    let sbr_concat_sources = vec![sbr_aac.clone(), sbr_aac];
    let sbr_concat_plan = fvid_media::plan_concat(&sbr_concat_sources, &sbr_opts)
        .expect("fvid_media::plan_concat should succeed for 1-packet SBR prefix across 2 segments");
    assert!(
        sbr_concat_plan.steps[0].detail.contains("48000 Hz, 1 channels"),
        "expected negotiated 48000 Hz in plan_concat step, got: {}",
        sbr_concat_plan.steps[0].detail
    );
    assert!(
        sbr_concat_plan
            .notes
            .iter()
            .any(|n| n.contains("encoded packet limit: 24 bytes; packet work limit: 1 packets")),
        "expected budget note in fvid_media::plan_concat, got: {:?}",
        sbr_concat_plan.notes
    );
}
