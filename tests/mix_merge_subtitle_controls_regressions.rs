use std::path::{Path, PathBuf};
use std::process::Command;

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-mix-merge-subtitle-controls-{name}-{}",
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

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn mix_audio_merge_audio_and_convert_subtitles_honor_budget_and_progress_controls() {
    let wav_a = fixture("mix-merge-controls-a.wav");
    let wav_b = fixture("mix-merge-controls-b.wav");
    let video = fixture("mix-merge-controls-video.y4m");
    let srt = fixture("mix-merge-controls.srt");
    let tmp = Directory::new("controls");

    assert_eq!(fvid::media::decode_video(&video).unwrap().video_frames, 2);

    // 1. CLI plan mix-audio & mix-audio with --max-rss-mib, --max-memory-mib, --max-packet-bytes, --max-packets, --progress
    let plan_mix_ok = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "mix-audio",
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
            "--max-memory-mib",
            "64",
            "--max-packet-bytes",
            "65536",
            "--max-packets",
            "100",
        ])
        .output()
        .unwrap();
    assert!(
        plan_mix_ok.status.success(),
        "CLI plan mix-audio failed: {}",
        String::from_utf8_lossy(&plan_mix_ok.stderr)
    );

    let plan_mix_rss_err = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "mix-audio",
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !plan_mix_rss_err.status.success(),
        "CLI plan mix-audio should refuse 1 MiB RSS budget"
    );

    let mix_out = tmp.0.join("mix.wav");
    let mix_rss_err = Command::new(bin_path())
        .args([
            "media",
            "mix-audio",
            mix_out.to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!mix_rss_err.status.success());
    assert!(!mix_out.exists());

    let mix_ok = Command::new(bin_path())
        .args([
            "media",
            "mix-audio",
            mix_out.to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
            "--max-memory-mib",
            "64",
            "--max-packet-bytes",
            "65536",
            "--max-packets",
            "100",
            "--progress",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        mix_ok.status.success(),
        "CLI mix-audio failed: {}",
        String::from_utf8_lossy(&mix_ok.stderr)
    );
    assert!(mix_out.is_file());
    assert!(
        String::from_utf8_lossy(&mix_ok.stderr).contains("\"done\":true"),
        "expected progress JSON on stderr, got: {}",
        String::from_utf8_lossy(&mix_ok.stderr)
    );

    // 2. CLI plan merge-audio & merge-audio with --max-rss-mib, --max-memory-mib, --max-packet-bytes, --max-packets, --progress
    let plan_merge_ok = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "merge-audio",
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
            "--max-memory-mib",
            "64",
            "--max-packet-bytes",
            "65536",
            "--max-packets",
            "100",
        ])
        .output()
        .unwrap();
    assert!(
        plan_merge_ok.status.success(),
        "CLI plan merge-audio failed: {}",
        String::from_utf8_lossy(&plan_merge_ok.stderr)
    );

    let plan_merge_rss_err = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "merge-audio",
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !plan_merge_rss_err.status.success(),
        "CLI plan merge-audio should refuse 1 MiB RSS budget"
    );

    let merge_out = tmp.0.join("merge.wav");
    let merge_rss_err = Command::new(bin_path())
        .args([
            "media",
            "merge-audio",
            merge_out.to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!merge_rss_err.status.success());
    assert!(!merge_out.exists());

    let merge_ok = Command::new(bin_path())
        .args([
            "media",
            "merge-audio",
            merge_out.to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
            "--max-memory-mib",
            "64",
            "--max-packet-bytes",
            "65536",
            "--max-packets",
            "100",
            "--progress",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        merge_ok.status.success(),
        "CLI merge-audio failed: {}",
        String::from_utf8_lossy(&merge_ok.stderr)
    );
    assert!(merge_out.is_file());
    assert!(
        String::from_utf8_lossy(&merge_ok.stderr).contains("\"done\":true"),
        "expected progress JSON on stderr, got: {}",
        String::from_utf8_lossy(&merge_ok.stderr)
    );

    // 3. CLI convert-subtitles with --max-packets, --max-packet-bytes, --max-rss-mib, --progress
    let subs_cap_out = tmp.0.join("subs-cap.mkv");
    let subs_cap_err = Command::new(bin_path())
        .args([
            "media",
            "convert-subtitles",
            srt.to_str().unwrap(),
            subs_cap_out.to_str().unwrap(),
            "--max-packets",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !subs_cap_err.status.success(),
        "convert-subtitles should refuse 2-cue SRT when --max-packets 1"
    );
    assert!(!subs_cap_out.exists());

    let subs_rss_err = Command::new(bin_path())
        .args([
            "media",
            "convert-subtitles",
            srt.to_str().unwrap(),
            subs_cap_out.to_str().unwrap(),
            "--max-rss-mib",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !subs_rss_err.status.success(),
        "convert-subtitles should refuse 1 MiB RSS budget"
    );
    assert!(!subs_cap_out.exists());

    let subs_ok_out = tmp.0.join("subs-ok.mkv");
    let subs_ok = Command::new(bin_path())
        .args([
            "media",
            "convert-subtitles",
            srt.to_str().unwrap(),
            subs_ok_out.to_str().unwrap(),
            "--max-packets",
            "10",
            "--max-packet-bytes",
            "4096",
            "--max-rss-mib",
            "4096",
            "--progress",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        subs_ok.status.success(),
        "CLI convert-subtitles failed: {}",
        String::from_utf8_lossy(&subs_ok.stderr)
    );
    assert!(subs_ok_out.is_file());
    assert!(
        String::from_utf8_lossy(&subs_ok.stderr).contains("\"done\":true"),
        "expected progress JSON on stderr, got: {}",
        String::from_utf8_lossy(&subs_ok.stderr)
    );
}
