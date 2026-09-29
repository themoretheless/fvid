use fvid::container::{
    mp4::{Limits, Mp4Reader},
    mp4_relocate::fast_start,
};
use std::io::Cursor;
fn verify(source: &[u8]) {
    let mut output = Vec::new();
    fast_start(&mut Cursor::new(source), &mut output).unwrap();
    assert_eq!(source.len(), output.len());
    assert!(
        output.windows(4).position(|s| s == b"moov").unwrap()
            < output.windows(4).position(|s| s == b"mdat").unwrap()
    );
    let mut before = Mp4Reader::open(Cursor::new(source), Limits::default()).unwrap();
    let mut after = Mp4Reader::open(Cursor::new(&output), Limits::default()).unwrap();
    assert_eq!(before.tracks().len(), after.tracks().len());
    for index in 0..before.tracks().len() {
        let a = &before.tracks()[index];
        let b = &after.tracks()[index];
        assert_eq!(
            (a.codec, a.timescale, a.duration, a.rotation, a.pixel_aspect),
            (b.codec, b.timescale, b.duration, b.rotation, b.pixel_aspect)
        );
        assert_eq!(a.configuration, b.configuration);
        assert_eq!((&a.name, &a.language), (&b.name, &b.language));
        assert_eq!(
            a.edits
                .iter()
                .map(|e| (e.duration, e.media_time))
                .collect::<Vec<_>>(),
            b.edits
                .iter()
                .map(|e| (e.duration, e.media_time))
                .collect::<Vec<_>>()
        );
        assert_eq!(a.samples.len(), b.samples.len());
        for packet in 0..a.samples.len() {
            let x = before.tracks()[index].samples.get(packet).unwrap();
            let y = after.tracks()[index].samples.get(packet).unwrap();
            assert_eq!(
                (x.size, x.dts, x.pts, x.duration, x.sync),
                (y.size, y.dts, y.pts, y.duration, y.sync)
            );
            let (mut left, mut right) = (Vec::new(), Vec::new());
            before.read_packet(index, packet, &mut left).unwrap();
            after.read_packet(index, packet, &mut right).unwrap();
            assert_eq!(left, right);
        }
    }
    let mut again = Vec::new();
    fast_start(&mut Cursor::new(&output), &mut again).unwrap();
    assert_eq!(output, again);
}
#[test]
fn avc_hevc_aac_tables_and_packets_survive_relocation() {
    for source in [
        include_bytes!("fixtures/avc/hlg-vui-only.mp4").as_slice(),
        include_bytes!("fixtures/hevc/main10-ipb.mp4").as_slice(),
        include_bytes!("fixtures/audio/aac-native-edit.m4a").as_slice(),
        include_bytes!("fixtures/audio/two-audio.mp4").as_slice(),
        include_bytes!("fixtures/subtitles/mov-text-tracks.mp4").as_slice(),
    ] {
        verify(source);
    }
    let mut co64 = Vec::new();
    fvid::container::mp4_write::write_adts_aac(
        include_bytes!("fixtures/audio/aac-stereo.aac"),
        &mut co64,
    )
    .unwrap();
    verify(&co64);
}
#[test]
fn invalid_offsets_and_fragmented_inputs_fail_before_output() {
    let mut corrupted = include_bytes!("fixtures/audio/aac-native-edit.m4a").to_vec();
    let table = corrupted.windows(4).position(|v| v == b"stco").unwrap();
    corrupted[table + 12..table + 16].copy_from_slice(&1u32.to_be_bytes());
    for source in [
        corrupted.as_slice(),
        include_bytes!("fixtures/video.mp4").as_slice(),
    ] {
        let mut output = Vec::new();
        assert!(fast_start(&mut Cursor::new(source), &mut output).is_err());
        assert!(output.is_empty());
    }
}

#[test]
fn cli_relocation_preserves_destination_and_cleans_failure() {
    let dir = std::env::temp_dir().join(format!("fvid-relocation-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let output = dir.join("out.mp4");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let bytes = std::fs::read(&output).unwrap();
    assert!(fvid::native_export::remux_mp4(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    let fragmented =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    assert!(fvid::native_export::remux_mp4(&fragmented, &dir.join("failed.mp4")).is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
