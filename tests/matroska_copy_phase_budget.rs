#![cfg(feature = "player")]
use std::{
    io::Cursor,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-copy-phase-budget/three-colors.mkv")
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn disjoint_inspection_and_copy_phases_fit_the_existing_seventy_kib_budget() {
    let source = source();
    let data = std::fs::read(&source).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(data.clone()), 32 << 20).unwrap();
    let rgb: Vec<_> = (0..3)
        .map(|_| {
            video
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect();
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut index =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(data.clone()), Default::default())
            .unwrap();
    index.scan_all().unwrap();
    assert_eq!(index.packets.len(), 3);
    assert!(index.estimated_index_payload_bytes().unwrap() < 70 << 10);
    let options = fvid_control::CopyOptions {
        max_controlled_bytes: Some(70 << 10),
        ..Default::default()
    };
    fvid_media::plan_remux(&source, &options).unwrap();
    fvid::native_plan::remux_with_options(&source, &options)
        .unwrap()
        .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "fvid-copy-phase-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let dir = Cleanup(dir);
    let out = dir.0.join("copy.mkv");
    let event = fvid_media::remux(&source, &out, &options).unwrap();
    assert_eq!(event.packets, 3);
    assert_eq!(std::fs::read(&out).unwrap(), data);
    let mut copy = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(out).unwrap()),
        32 << 20,
    )
    .unwrap();
    for expected in rgb {
        assert_eq!(
            copy.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            expected
        );
    }
    assert!(copy.read_frame_raw().unwrap().is_none());
}
#[test]
fn genuinely_small_budgets_still_refuse_before_copying() {
    let data = std::fs::read(source()).unwrap();
    for budget in [200, 64 << 10] {
        let mut out = Vec::new();
        let error = fvid_media::owned_matroska_copy::copy_with_packet_and_memory_limits(
            &mut Cursor::new(&data),
            &mut out,
            false,
            None,
            None,
            32 << 20,
            Some(budget),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("controlled memory budget exceeded"),
            "{error}"
        );
        assert!(out.is_empty());
    }
}
