use std::{io::Cursor, path::PathBuf};
struct Clean(PathBuf);
impl Drop for Clean { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
#[test]
fn synthetic_srt_public_library_conversion_matches_frontend_and_refuses_overwrite() {
    let dir = std::env::temp_dir().join(format!("fvid-owned-subtitle-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let _clean = Clean(dir.clone());
    let source = dir.join("source.srt");
    std::fs::write(&source, "1\n00:00:00,125 --> 00:00:01,250\n<i>Synthetic</i> &amp; text\n\n2\n00:00:02,000 --> 00:00:02,500\nSecond cue\n").unwrap();
    let expected = dir.join("expected.mkv");
    fvid::native_subtitle::try_convert(&source, &expected, &[]).unwrap().unwrap();
    let output = dir.join("library.mkv");
    let options = fvid_media_info::SubtitleConvertOptions::default();
    let stats = fvid_media::convert_subtitles(&source, &output, &options).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.cues, 2);
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(bytes, std::fs::read(expected).unwrap());
    let mut reader = fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes.clone()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].codec, "S_TEXT/ASS");
    assert_eq!(reader.packets[0].pts_ns, 125_000_000);
    assert_eq!(reader.packets[0].duration_ns, Some(1_125_000_000));
    assert!(fvid_media::convert_subtitles(&source, &output, &options).is_err());
    assert_eq!(std::fs::read(output).unwrap(), bytes);
    let invalid = dir.join("invalid.mkv");
    let options = fvid_media_info::SubtitleConvertOptions { streams: vec![1], ..Default::default() };
    assert!(fvid_media::convert_subtitles(&source, &invalid, &options).is_err());
    assert!(!invalid.exists());
}
