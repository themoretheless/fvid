use fvid::{
    container::{mp4_concat, webm::WebmReader},
    media::CopyOptions,
};
use std::{io::Cursor, path::PathBuf};
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-concat-selected-clock/video-and-long-audio.mp4")
}
#[test]
fn unselected_audio_does_not_insert_video_concat_gap() {
    let source = source();
    let options = CopyOptions {
        streams: vec![0],
        ..Default::default()
    };
    let mut segments = mp4_concat::open_with_options(&[source.clone(), source], &options)
        .unwrap()
        .unwrap();
    let mut out = Cursor::new(Vec::new());
    assert_eq!(
        mp4_concat::write_with_options(&mut segments, &mut out, &options)
            .unwrap()
            .packets,
        2
    );
    let mut reader = WebmReader::open(Cursor::new(out.into_inner()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks.len(), 1);
    assert_eq!(reader.packets.len(), 2);
    assert_eq!(reader.packets[0].pts_ns, 0);
    assert_eq!(
        reader.packets[1].pts_ns, 40_000_000,
        "unselected long audio must not stretch the selected video segment"
    );
}
#[cfg(feature = "player")]
#[test]
fn selected_video_plays_seeks_rewinds_and_ends_at_eighty_ms() {
    use fvid::playback_native::NativeReader;
    use std::{fs::File, io::BufReader, time::Duration};
    let source = source();
    let mut original =
        NativeReader::software(BufReader::new(File::open(&source).unwrap()), 32 << 20).unwrap();
    let expected = original
        .read_frame_raw()
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    let options = CopyOptions {
        streams: vec![0],
        ..Default::default()
    };
    let mut segments = mp4_concat::open_with_options(&[source.clone(), source], &options)
        .unwrap()
        .unwrap();
    let mut output = Cursor::new(Vec::new());
    mp4_concat::write_with_options(&mut segments, &mut output, &options).unwrap();
    let mut video =
        NativeReader::software(BufReader::new(Cursor::new(output.into_inner())), 32 << 20).unwrap();
    assert_eq!(video.duration(), Some(Duration::from_millis(80)));
    for _ in 0..2 {
        assert_eq!(
            video
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            expected
        );
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    assert!(video.read_frame_raw().unwrap().is_none());
    video.rewind().unwrap();
    assert_eq!(
        video
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        expected
    );
    for ms in [0, 39, 40, 79, 80, 100] {
        assert_eq!(
            video
                .seek_raw(Duration::from_millis(ms))
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            expected
        );
        if ms >= 80 {
            assert!(video.read_frame_raw().unwrap().is_none());
        }
    }
}
