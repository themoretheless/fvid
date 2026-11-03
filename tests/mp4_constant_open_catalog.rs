use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
#[test]
fn constant_open_keeps_all_packets_catalogs_and_compact_sample_budget() {
    for name in [
        "catalog1-selected1.mp4",
        "catalog32-selected1.mp4",
        "catalog32-selected32.mp4",
    ] {
        let bytes = std::fs::read(root().join("mp4-admission-catalog").join(name)).unwrap();
        let mut owned = fvid_media::owned_mp4::Mp4Reader::open(
            Cursor::new(bytes.clone()),
            fvid_media::owned_mp4::Limits {
                samples: 4,
                ..Default::default()
            },
        )
        .unwrap();
        let mut native = fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes),
            fvid::container::mp4::Limits {
                samples: 4,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(owned.tracks().len(), 2);
        assert_eq!(native.tracks().len(), 2);
        assert_eq!(owned.tracks()[1].samples.len(), 16384);
        assert_eq!(owned.tracks()[1].samples.cost(), 1);
        assert_eq!(native.tracks()[1].samples.cost(), 1);
        assert_eq!(
            owned.tracks()[1].sample_descriptions.len(),
            if name.starts_with("catalog1-") { 1 } else { 32 }
        );
        let mut a = Vec::new();
        let mut b = Vec::new();
        for track in 0..2 {
            let count = owned.tracks()[track].samples.len();
            assert_eq!(native.tracks()[track].samples.len(), count);
            for i in 0..count {
                let x = owned.tracks()[track].samples.get(i).unwrap();
                let y = native.tracks()[track].samples.get(i).unwrap();
                assert_eq!(
                    (
                        x.description_index,
                        x.offset,
                        x.size,
                        x.dts,
                        x.pts,
                        x.duration,
                        x.sync
                    ),
                    (
                        y.description_index,
                        y.offset,
                        y.size,
                        y.dts,
                        y.pts,
                        y.duration,
                        y.sync
                    )
                );
                owned.read_packet(track, i, &mut a).unwrap();
                native.read_packet(track, i, &mut b).unwrap();
                assert_eq!(a, b);
                if track == 1 {
                    assert_eq!(a, [0, 16]);
                }
            }
        }
    }
}
#[test]
fn actually_selected_changed_headers_keep_constant_api_refusals() {
    let input = std::fs::read(root().join("mp4-uniform-plan-scan/control-endian.mp4")).unwrap();
    let indexed =
        fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&input), Default::default())
            .unwrap();
    assert_eq!(indexed.tracks()[1].samples.len(), 64);
    let owned =
        match fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&input), Default::default()) {
            Err(e) => e,
            Ok(_) => panic!("changed setup admitted"),
        };
    assert!(owned.is_unsupported());
    assert!(owned.to_string().contains("decoder reconfiguration"));
    let native = match fvid::container::mp4::Mp4Reader::open(Cursor::new(input), Default::default())
    {
        Err(e) => e,
        Ok(_) => panic!("changed setup admitted"),
    };
    assert!(matches!(native, fvid::Error::Unsupported(_)));
    assert!(native.to_string().contains("decoder reconfiguration"));
}
