use fvid::media::{self, CopyOptions, SubtitleConvertOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-subtitle-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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
fn subtitle_plan_and_convert_single_pass_for_srt_ass_and_matroska() {
    let dir = TempDir::new();
    let srt_path = dir.0.join("sample.srt");
    std::fs::write(
        &srt_path,
        "1\r\n00:00:00,000 --> 00:00:01,500\r\nHello <b>world</b>\r\n\r\n2\n00:00:01,500 --> 00:00:03,000\nLine two\n",
    )
    .unwrap();

    let options = SubtitleConvertOptions::default();
    let controls = CopyOptions::default();

    let srt_plan = media::plan_convert_subtitles_with_controls(&srt_path, &options, &controls)
        .expect("SRT subtitle plan should succeed");
    assert_eq!(srt_plan.command, "convert-subtitles");
    assert_eq!(srt_plan.streams[0].codec, "subrip");

    let mkv_path = dir.0.join("sample.mkv");
    let srt_stats = media::convert_subtitles_with_controls(&srt_path, &mkv_path, &options, &controls)
        .expect("SRT subtitle conversion should succeed");
    assert_eq!(srt_stats.cues, 2);

    let mkv_plan = media::plan_convert_subtitles_with_controls(&mkv_path, &options, &controls)
        .expect("Matroska ASS subtitle plan should succeed in a single WebmReader scan");
    assert_eq!(mkv_plan.command, "convert-subtitles");
    assert_eq!(mkv_plan.streams[0].codec, "ass");
}
