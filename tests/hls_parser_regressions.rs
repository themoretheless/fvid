use fvid_media::owned_hls::{self, Playlist};
use std::{
    fs,
    io::{self, Cursor, Write},
    path::PathBuf,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hls-parser")
}
fn text(name: &str) -> String {
    fs::read_to_string(root().join(name)).unwrap()
}

#[test]
fn control_assembles_to_the_decodable_synthetic_video() {
    let text = text("control.m3u8");
    let Playlist::Media(playlist) = owned_hls::parse(&text).unwrap() else {
        panic!("media")
    };
    let mut output = Vec::new();
    owned_hls::assemble_fmp4(
        &playlist,
        &mut output,
        |resource| {
            Ok(Box::new(Cursor::new(
                fs::read(root().join(resource.uri)).unwrap(),
            )))
        },
        None,
        None,
    )
    .unwrap();
    assert_eq!(output, fs::read(root().join("control.mp4")).unwrap());
    let mut playback = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(output),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    let mut count = 0;
    while playback.read_frame().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 25);
}

#[test]
fn duplicate_singleton_declarations_are_refused_before_assembly() {
    for name in [
        "version",
        "target-duration",
        "media-sequence",
        "endlist",
        "independent-segments",
    ] {
        let text = text(&format!("duplicate-{name}.m3u8"));
        let error = owned_hls::parse(&text).expect_err("duplicate singleton must fail");
        assert!(
            error.contains("duplicate HLS singleton tag"),
            "{name}: {error}"
        );
    }
}

#[test]
fn master_and_media_tags_cannot_silently_mix() {
    for name in ["mixed-media-rendition.m3u8", "mixed-master-sequence.m3u8"] {
        let text = text(name);
        let error = owned_hls::parse(&text).expect_err("mixed tag classes must fail");
        assert_eq!(error, "mixed HLS master and media playlist", "{name}");
    }
}

#[test]
fn protocol_decimal_integers_do_not_accept_a_leading_plus() {
    for name in [
        "signed-target.m3u8",
        "signed-sequence.m3u8",
        "signed-bandwidth.m3u8",
        "exponent-duration.m3u8",
        "signed-duration.m3u8",
        "signed-range-length.m3u8",
        "signed-range-offset.m3u8",
    ] {
        let text = text(name);
        assert!(owned_hls::parse(&text).is_err(), "{name}");
    }
}

#[test]
fn cancelling_in_the_final_write_does_not_report_success() {
    struct CancellingWriter {
        data: Vec<u8>,
        limit: usize,
        flag: fvid_control::CancelFlag,
    }
    impl Write for CancellingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.data.extend_from_slice(bytes);
            if self.data.len() == self.limit {
                self.flag.cancel();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let text = text("control.m3u8");
    let Playlist::Media(playlist) = owned_hls::parse(&text).unwrap() else {
        panic!("media")
    };
    let flag = fvid_control::CancelFlag::new();
    let mut writer = CancellingWriter {
        data: Vec::new(),
        limit: fs::metadata(root().join("control.mp4")).unwrap().len() as usize,
        flag: flag.clone(),
    };
    let error = owned_hls::assemble_fmp4(
        &playlist,
        &mut writer,
        |resource| {
            Ok(Box::new(Cursor::new(
                fs::read(root().join(resource.uri)).unwrap(),
            )))
        },
        None,
        Some(&flag),
    )
    .expect_err("late cancellation must fail");
    assert_eq!(error, "HLS assembly cancelled");
    assert_eq!(writer.data, fs::read(root().join("control.mp4")).unwrap());
}
