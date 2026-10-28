use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};
use fvid::media_info::NormalizeTarget;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-concat-packet-prefix-budget")
        .join(name)
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch266-{}-{}-{}",
        tag,
        std::process::id(),
        nanos
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mp4_single_source_audio_operations_scope_packet_byte_budget_to_max_packets_prefix_on_open() {
    let seg_b = fixture("seg-b-mixed.m4a");
    let seg_a = fixture("seg-a-small.m4a");
    let tmp = temp_dir("single-mp4-open");

    // seg-b-mixed.m4a has packet 0 (8 bytes) and packet 1 (48 bytes).
    // With max_packet_bytes = 16 and max_packets = Some(1), packet 0 is within budget
    // and packet 1 is outside the active prefix.
    // Previously, Mp4Reader::open / Mp4Reader::open_indexed checked all stsz sample sizes
    // against options.max_packet_bytes during moov parsing before max_packets could bound the prefix.
    let prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(1),
        ..CopyOptions::default()
    };
    let controlled_prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(1),
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..CopyOptions::default()
    };

    for (label, opts) in [
        ("uncontrolled", &prefix_opts),
        ("controlled", &controlled_prefix_opts),
    ] {
        let decode_plan = media::plan_decode_audio(
            &seg_b,
            &AudioDecodeTransform::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("plan_decode_audio ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(decode_plan.streams.len(), 1);

        let wav = tmp.join(format!("decode-{label}.wav"));
        let decode_stats = media::decode_audio_transformed(
            &seg_b,
            &wav,
            AudioDecodeTransform::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("decode_audio_transformed ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(decode_stats.decoded_frames, 1);
        assert_eq!(decode_stats.sample_frames, 1024);

        let mka = tmp.join(format!("decode-{label}.mka"));
        let mka_stats = media::decode_audio_transformed(
            &seg_b,
            &mka,
            AudioDecodeTransform::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("decode_audio_transformed to .mka ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(mka_stats.decoded_frames, 1);
        assert_eq!(mka_stats.sample_frames, 1024);

        let loud_plan = media::plan_loudness(&seg_b, opts)
            .unwrap_or_else(|e| panic!("plan_loudness ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(loud_plan.streams.len(), 1);

        let loud_report = media::measure_loudness(&seg_b, opts)
            .unwrap_or_else(|e| panic!("measure_loudness ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(loud_report.sample_frames, 1024);

        let norm_plan = media::plan_normalize_loudness(
            &seg_b,
            opts,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_or_else(|e| panic!("plan_normalize_loudness ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(norm_plan.streams.len(), 1);

        let ln_plan = media::plan_loudnorm(&seg_b, None, false, opts)
            .unwrap_or_else(|e| panic!("plan_loudnorm ({label}) on 1-packet prefix should succeed: {e}"));
        assert!(!ln_plan.steps.is_empty());

        let ln_wav = tmp.join(format!("ln-{label}.wav"));
        let ln_stats = media::apply_loudnorm(&seg_b, &ln_wav, None, opts)
            .unwrap_or_else(|e| panic!("apply_loudnorm ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(ln_stats.sample_rate, 192_000);
        assert_eq!(ln_stats.sample_frames, 4096);

        let mix_inputs = vec![seg_b.clone(), seg_a.clone()];
        let mix_plan = media::plan_mix_audio_with_controls(
            &mix_inputs,
            &MixAudioOptions::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("plan_mix_audio_with_controls ({label}) on 1-packet prefix should succeed: {e}"));
        assert!(!mix_plan.steps.is_empty());

        let mix_wav = tmp.join(format!("mix-{label}.wav"));
        let mix_stats = media::mix_audio_with_controls(
            &mix_inputs,
            &mix_wav,
            &MixAudioOptions::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("mix_audio_with_controls ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(mix_stats.sample_frames, 1024);

        let merge_plan = media::plan_merge_audio_with_controls(&mix_inputs, opts)
            .unwrap_or_else(|e| panic!("plan_merge_audio_with_controls ({label}) on 1-packet prefix should succeed: {e}"));
        assert!(!merge_plan.steps.is_empty());

        let merge_wav = tmp.join(format!("merge-{label}.wav"));
        let merge_stats = media::merge_audio_with_controls(&mix_inputs, &merge_wav, opts)
            .unwrap_or_else(|e| panic!("merge_audio_with_controls ({label}) on 1-packet prefix should succeed: {e}"));
        assert_eq!(merge_stats.channels, 2);
        assert_eq!(merge_stats.sample_frames, 1024);
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn sample_domain_concat_scopes_packet_byte_budget_to_cumulative_max_packets_prefix() {
    let m4a_a = fixture("seg-a-small.m4a");
    let m4a_b = fixture("seg-b-mixed.m4a");
    let mka_a = fixture("seg-a-small.mka");
    let mka_b = fixture("seg-b-mixed.mka");
    let tmp = temp_dir("concat-prefix");

    // Case 1: Mixed container concat [seg-a-small.m4a (2 x 8B), seg-b-mixed.mka (8B, 48B)]
    // and [seg-a-small.mka (2 x 8B), seg-b-mixed.m4a (8B, 48B)], as well as homogeneous
    // Matroska AAC concat [seg-a-small.mka, seg-b-mixed.mka] and MP4->WAV concat [seg-a-small.m4a, seg-b-mixed.m4a].
    for (case_label, inputs) in [
        ("mka-pair", vec![mka_a.clone(), mka_b.clone()]),
        ("m4a-then-mka", vec![m4a_a.clone(), mka_b.clone()]),
        ("mka-then-m4a", vec![mka_a.clone(), m4a_b.clone()]),
    ] {
        // 1a. max_packets = Some(1): stops after packet 0 of segment 0 (1024 frames); segment 1 is never reached.
        let opts_1 = CopyOptions {
            max_packet_bytes: 16,
            max_packets: Some(1),
            ..CopyOptions::default()
        };
        let plan_1 = media::plan_concat(&inputs, &opts_1)
            .unwrap_or_else(|e| panic!("plan_concat ({case_label}, max_packets=1) should succeed: {e}"));
        assert_eq!(plan_1.streams.len(), 1);

        for ext in ["wav", "mka"] {
            let out_1 = tmp.join(format!("{case_label}-max1.{ext}"));
            let stats_1 = media::concat(&inputs, &out_1, &opts_1)
                .unwrap_or_else(|e| panic!("concat ({case_label}, max_packets=1, .{ext}) should succeed: {e}"));
            assert_eq!(stats_1.packets, 1);
            let out_report_1 = media::measure_loudness(&out_1, &CopyOptions::default()).unwrap();
            assert_eq!(out_report_1.sample_frames, 1024);
        }

        // 1b. max_packets = Some(3): consumes 2 packets from segment 0 and 1 packet from segment 1 (3072 frames total),
        // stopping before the 48-byte packet 1 of segment 1!
        let opts_3 = CopyOptions {
            max_packet_bytes: 16,
            max_packets: Some(3),
            ..CopyOptions::default()
        };
        let plan_3 = media::plan_concat(&inputs, &opts_3)
            .unwrap_or_else(|e| panic!("plan_concat ({case_label}, max_packets=3) should succeed: {e}"));
        assert_eq!(plan_3.streams.len(), 1);

        for ext in ["wav", "mka"] {
            let out_3 = tmp.join(format!("{case_label}-max3.{ext}"));
            let stats_3 = media::concat(&inputs, &out_3, &opts_3)
                .unwrap_or_else(|e| panic!("concat ({case_label}, max_packets=3, .{ext}) should succeed: {e}"));
            assert_eq!(stats_3.packets, 3);
            let out_report_3 = media::measure_loudness(&out_3, &CopyOptions::default()).unwrap();
            assert_eq!(out_report_3.sample_frames, 3072);
        }

        // 1c. max_packets = Some(4) or None: reaches the 48-byte packet 1 of segment 1, which exceeds max_packet_bytes = 16!
        for max_packets in [Some(4), None] {
            let opts_fail = CopyOptions {
                max_packet_bytes: 16,
                max_packets,
                ..CopyOptions::default()
            };
            let plan_err = media::plan_concat(&inputs, &opts_fail)
                .expect_err(&format!("plan_concat ({case_label}, max_packets={max_packets:?}) must reject 48-byte packet"))
                .to_string();
            assert!(
                plan_err.contains("packet exceeds budget"),
                "unexpected plan_concat error ({case_label}, max_packets={max_packets:?}): {plan_err}"
            );

            for ext in ["wav", "mka"] {
                let out_fail = tmp.join(format!("{case_label}-fail-{max_packets:?}.{ext}"));
                let exec_err = media::concat(&inputs, &out_fail, &opts_fail)
                    .expect_err(&format!("concat ({case_label}, max_packets={max_packets:?}, .{ext}) must reject 48-byte packet"))
                    .to_string();
                assert!(
                    exec_err.contains("packet exceeds budget"),
                    "unexpected concat error ({case_label}, max_packets={max_packets:?}, .{ext}): {exec_err}"
                );
                assert!(!out_fail.exists(), "failed concat must not publish output");
            }
        }
    }

    // Case 2: Homogeneous MP4 concat into .wav (sample-domain concat route via native_audio_mix::concat)
    let m4a_inputs = vec![m4a_a.clone(), m4a_b.clone()];
    for (max_packets, expected_frames) in [(Some(1), 1024u64), (Some(2), 2048u64), (Some(3), 3072u64)] {
        let opts = CopyOptions {
            max_packet_bytes: 16,
            max_packets,
            ..CopyOptions::default()
        };
        let out_wav = tmp.join(format!("m4a-pair-max{}.wav", max_packets.unwrap()));
        let stats = media::concat(&m4a_inputs, &out_wav, &opts)
            .unwrap_or_else(|e| panic!("MP4->WAV concat (max_packets={max_packets:?}) should succeed: {e}"));
        assert_eq!(stats.packets, max_packets.unwrap());
        let out_report = media::measure_loudness(&out_wav, &CopyOptions::default()).unwrap();
        assert_eq!(out_report.sample_frames, expected_frames);
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
