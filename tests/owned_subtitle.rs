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

#[test]
fn synthetic_ass_and_matroska_subrip_public_conversion_preserves_cues() {
    fn element(id: &[u8], data: &[u8]) -> Vec<u8> {
        [id, &(data.len() as u32 | 0x1000_0000).to_be_bytes(), data].concat()
    }
    let dir = std::env::temp_dir().join(format!("fvid-owned-subtitle-containers-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let _clean = Clean(dir.clone());
    let track = element(&[0xae], &[
        element(&[0xd7], &[1]), element(&[0x83], &[17]),
        element(&[0x86], b"S_TEXT/UTF8"), element(&[0x53, 0x6e], b"Synthetic"),
        element(&[0x22, 0xb5, 0x9c], b"eng"),
    ].concat());
    let group = element(&[0xa0], &[
        element(&[0xa1], &[&[0x81, 0, 0, 0x80][..], b"<b>Text</b> &amp; cue"].concat()),
        element(&[0x9b], &1_125_000_000u64.to_be_bytes()),
    ].concat());
    let container = [
        element(&[0x1a, 0x45, 0xdf, 0xa3], &element(&[0x42, 0x82], b"matroska")),
        element(&[0x18, 0x53, 0x80, 0x67], &[
            element(&[0x15, 0x49, 0xa9, 0x66], &element(&[0x2a, 0xd7, 0xb1], &[1])),
            element(&[0x16, 0x54, 0xae, 0x6b], &track),
            element(&[0x1f, 0x43, 0xb6, 0x75], &[
                element(&[0xe7], &125_000_000u64.to_be_bytes()), group,
            ].concat()),
        ].concat()),
    ].concat();
    let source = dir.join("subrip.mkv");
    std::fs::write(&source, container).unwrap();
    let output = dir.join("ass.mkv");
    let options = fvid_media_info::SubtitleConvertOptions { streams: vec![0], ..Default::default() };
    let stats = fvid_media::convert_subtitles(&source, &output, &options).unwrap();
    assert_eq!((stats.backend, stats.cues), ("fvid", 1));
    let expected = dir.join("frontend.mkv");
    fvid::native_subtitle::try_convert_with_options(&source, &expected, &options).unwrap().unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), std::fs::read(expected).unwrap());
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&output).unwrap()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks[0].name, "Synthetic");
    assert_eq!(reader.tracks[0].language, "eng");
    assert_eq!(reader.packets[0].pts_ns, 125_000_000);
    assert_eq!(reader.packets[0].duration_ns, Some(1_125_000_000));
    assert_eq!(reader.read_packet(0).unwrap(), b"0,0,Default,,0,0,0,,{\\b1}Text{\\b0} & cue");
    let copied = dir.join("copied.mkv");
    fvid_media::convert_subtitles(&output, &copied, &options).unwrap();
    assert_eq!(std::fs::read(copied).unwrap(), std::fs::read(&output).unwrap());

    let mut ass = reader.tracks[0].codec_private.clone();
    ass.extend_from_slice(b"Dialogue: 0,0:00:00.12,0:00:01.25,Default,,0,0,0,,{\\b1}Standalone{\\b0}\n");
    let source = dir.join("standalone.ass");
    std::fs::write(&source, ass).unwrap();
    let output = dir.join("standalone.mkv");
    fvid_media::convert_subtitles(&source, &output, &options).unwrap();
    let expected = dir.join("standalone-frontend.mkv");
    fvid::native_subtitle::try_convert_with_options(&source, &expected, &options).unwrap().unwrap();
    assert_eq!(std::fs::read(output).unwrap(), std::fs::read(expected).unwrap());
}
