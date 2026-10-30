use std::{io::Cursor, path::Path};

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-chapter-versions")
            .join(name),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn chapters(c: &serde_json::Value) {
    let name = c["file"].as_str().unwrap();
    let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
        .unwrap();
    let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
        .unwrap();
    let expected: Vec<_> = c["starts_ns"]
        .as_array()
        .unwrap()
        .iter()
        .zip(c["titles"].as_array().unwrap())
        .map(|(t, s)| (t.as_u64().unwrap(), s.as_str().unwrap().to_owned()))
        .collect();
    assert_eq!(
        a.chapters()
            .iter()
            .map(|c| (c.start_ns, c.title.clone()))
            .collect::<Vec<_>>(),
        expected,
        "root {name}"
    );
    assert_eq!(
        b.chapters()
            .iter()
            .map(|c| (c.start_ns, c.title.clone()))
            .collect::<Vec<_>>(),
        expected,
        "owned {name}"
    );
}
#[test]
fn version_one_and_empty_or_truncated_lists_keep_their_existing_policy() {
    for c in cases().into_iter().filter(|c| c["version"] == 1) {
        chapters(&c);
    }
}
#[test]
fn version_zero_reads_its_own_count_and_utf8_titles() {
    for c in cases().into_iter().filter(|c| c["version"] == 0) {
        chapters(&c);
    }
}
#[cfg(feature = "player")]
#[test]
fn every_chapter_movie_decodes_exact_pictures_and_eof() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(name)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let frame = video.read_frame().unwrap().unwrap();
            assert_eq!(
                (frame.presentation_time.ticks, frame.duration.ticks),
                (i, 1)
            );
            let planes: Vec<_> = frame
                .picture
                .y
                .iter()
                .chain(&frame.picture.cb)
                .chain(&frame.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(planes, bytes(&format!("frame-{i}.yuv16")), "{name}");
        }
        assert!(video.read_frame().unwrap().is_none());
    }
}
#[cfg(feature = "player")]
#[test]
fn parsed_chapters_seek_to_their_exact_native_intervals() {
    use std::time::Duration;
    for c in cases().into_iter().filter(|c| c.get("kind").is_none()) {
        let name = c["file"].as_str().unwrap();
        let r = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
            .unwrap();
        assert_eq!(
            r.chapters().len(),
            3,
            "{name}: chapter navigation must be available"
        );
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for (i, ch) in r.chapters().iter().enumerate() {
            assert_eq!(ch.start_ns, i as u64 * 20_000_000);
            assert!(
                video
                    .seek_raw(Duration::from_nanos(ch.start_ns))
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                video.frame_interval(),
                Some((i as u128, i as u128 + 1, 50)),
                "{name}"
            );
        }
    }
}
