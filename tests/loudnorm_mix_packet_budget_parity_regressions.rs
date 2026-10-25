use std::path::{Path, PathBuf};
use std::process::Command;

use fvid::media::{self, CopyOptions, MixAudioOptions};

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-loudnorm-mix-packet-budget-parity-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudnorm-mix-packet-budget-parity")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn matroska_and_wave_audio_plans_enforce_max_packet_bytes_and_zero_packet_limits() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let mka_flac = dir.join("matroska-flac-48k.mka");
    let mka_pcm = dir.join("matroska-pcm-48k.mka");

    let tiny_pkt = CopyOptions {
        max_packet_bytes: 2,
        ..CopyOptions::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..CopyOptions::default()
    };

    // 1. WAVE stereo s16le has 4-byte sample frames; max_packet_bytes=2 cannot hold 1 frame.
    let wav_decode_err = fvid_media::plan_decode_audio(&wav, &Default::default(), &tiny_pkt)
        .expect_err("plan_decode_audio on stereo s16le WAVE must reject max_packet_bytes=2");
    assert!(
        wav_decode_err.contains("PCM packet limit cannot hold one sample frame"),
        "unexpected wav_decode_err: {wav_decode_err}"
    );
    let wav_loudnorm_err = media::plan_loudnorm(&wav, None, false, &tiny_pkt)
        .expect_err("plan_loudnorm on stereo s16le WAVE must reject max_packet_bytes=2");
    assert!(
        wav_loudnorm_err
            .to_string()
            .contains("PCM packet limit cannot hold one sample frame")
            || wav_loudnorm_err
                .to_string()
                .contains("packet byte limit is smaller than one PCM sample frame"),
        "unexpected wav_loudnorm_err: {wav_loudnorm_err}"
    );
    let wav_zero_err = media::plan_loudnorm(&wav, None, false, &zero_pkts)
        .expect_err("plan_loudnorm on WAVE must reject max_packets=Some(0)");
    assert!(
        wav_zero_err.to_string().contains("at least one")
            || wav_zero_err.to_string().contains("no decoded audio samples"),
        "unexpected wav_zero_err: {wav_zero_err}"
    );

    // 2. Matroska FLAC and Matroska PCM must enforce max_packet_bytes during plan_decode_audio,
    // plan_loudness, and plan_loudnorm even when max_controlled_bytes is None.
    for (name, path) in [("mka-flac", &mka_flac), ("mka-pcm", &mka_pcm)] {
        let decode_err = fvid_media::plan_decode_audio(path, &Default::default(), &tiny_pkt)
            .expect_err("plan_decode_audio on Matroska must reject tiny max_packet_bytes");
        assert!(
            decode_err.contains("packet") || decode_err.contains("exceeds"),
            "unexpected decode_err for {name}: {decode_err}"
        );
        let loudness_err = media::plan_loudness(path, &tiny_pkt)
            .expect_err("plan_loudness on Matroska must reject tiny max_packet_bytes");
        assert!(
            loudness_err.to_string().contains("packet")
                || loudness_err.to_string().contains("exceeds"),
            "unexpected loudness_err for {name}: {loudness_err}"
        );
        let loudnorm_err = media::plan_loudnorm(path, None, false, &tiny_pkt)
            .expect_err("plan_loudnorm on Matroska must reject tiny max_packet_bytes");
        assert!(
            loudnorm_err.to_string().contains("packet")
                || loudnorm_err.to_string().contains("exceeds"),
            "unexpected loudnorm_err for {name}: {loudnorm_err}"
        );
    }
}

#[test]
fn native_loudnorm_try_plan_enforces_packet_bytes_zero_packets_and_rss_across_containers() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let adts = dir.join("adts-mono-48k.aac");
    let mp4 = dir.join("mp4-pcm-48k.m4a");
    let mka_pcm = dir.join("matroska-pcm-48k.mka");
    let mka_flac = dir.join("matroska-flac-48k.mka");
    let tmp = Directory::new("loudnorm-parity");

    let tiny_pkt = CopyOptions {
        max_packet_bytes: 2,
        ..CopyOptions::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..CopyOptions::default()
    };
    let tiny_rss = CopyOptions {
        max_rss_bytes: Some(1),
        ..CopyOptions::default()
    };

    for (name, path) in [
        ("wav", &wav),
        ("adts", &adts),
        ("mp4", &mp4),
        ("mka-pcm", &mka_pcm),
        ("mka-flac", &mka_flac),
    ] {
        // try_plan and try_apply must both reject tiny_pkt, zero_pkts, and tiny_rss
        for (label, opts) in [
            ("tiny_pkt", &tiny_pkt),
            ("zero_pkts", &zero_pkts),
            ("tiny_rss", &tiny_rss),
        ] {
            assert!(
                fvid::native_loudnorm::try_plan(path, None, false, opts).is_err(),
                "try_plan must reject {label} on {name}"
            );
            let out_wav = tmp.0.join(format!("{name}-{label}.wav"));
            assert!(
                fvid::native_loudnorm::try_apply(path, &out_wav, None, false, opts).is_err(),
                "try_apply must reject {label} on {name}"
            );
            assert!(!out_wav.exists());
        }

        // CLI `media plan loudnorm` must also reject `--max-packet-bytes 2` and `--max-packets 0`
        let cli_pkt = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "loudnorm",
                path.to_str().unwrap(),
                "--max-packet-bytes",
                "2",
            ])
            .output()
            .expect("cli plan loudnorm tiny_pkt");
        assert!(
            !cli_pkt.status.success(),
            "CLI plan loudnorm should reject --max-packet-bytes 2 on {name}"
        );

        let cli_zero = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "loudnorm",
                path.to_str().unwrap(),
                "--max-packets",
                "0",
            ])
            .output()
            .expect("cli plan loudnorm zero_pkts");
        assert!(
            !cli_zero.status.success(),
            "CLI plan loudnorm should reject --max-packets 0 on {name}"
        );
    }
}

#[test]
fn mix_merge_and_concat_plans_enforce_packet_bytes_and_zero_packet_limits() {
    let dir = fixture_dir();
    let wav = dir.join("stereo-48k.wav");
    let adts = dir.join("adts-mono-48k.aac");
    let mp4 = dir.join("mp4-pcm-48k.m4a");
    let mka_pcm = dir.join("matroska-pcm-48k.mka");
    let mka_flac = dir.join("matroska-flac-48k.mka");

    let tiny_pkt = CopyOptions {
        max_packet_bytes: 2,
        ..CopyOptions::default()
    };
    let zero_pkts = CopyOptions {
        max_packets: Some(0),
        ..CopyOptions::default()
    };

    for (name, sources) in [
        ("wav-pair", vec![wav.clone(), wav.clone()]),
        ("adts-pair", vec![adts.clone(), adts.clone()]),
        ("mp4-pair", vec![mp4.clone(), mp4.clone()]),
        ("mka-pcm-pair", vec![mka_pcm.clone(), mka_pcm.clone()]),
        ("mka-flac-pair", vec![mka_flac.clone(), mka_flac.clone()]),
    ] {
        for (label, opts) in [("tiny_pkt", &tiny_pkt), ("zero_pkts", &zero_pkts)] {
            assert!(
                media::plan_mix_audio_with_controls(&sources, &MixAudioOptions::default(), opts)
                    .is_err(),
                "plan_mix_audio_with_controls must reject {label} on {name}"
            );
            assert!(
                media::plan_merge_audio_with_controls(&sources, opts).is_err(),
                "plan_merge_audio_with_controls must reject {label} on {name}"
            );
            assert!(
                !fvid::native_audio_mix::eligible_with_options(&sources, opts).unwrap_or(false),
                "native_audio_mix::eligible_with_options must reject {label} on {name}"
            );
            assert!(
                !fvid::native_audio_mix::concat_eligible_with_options(&sources, opts)
                    .unwrap_or(false),
                "concat_eligible_with_options must reject {label} on {name}"
            );
        }
    }
}
