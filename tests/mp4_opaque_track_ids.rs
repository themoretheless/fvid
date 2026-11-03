use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-opaque-track-ids")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn unique_track_ids_keep_opaque_skips_and_exact_v0_v1_video_playback() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let total = c["totalTracks"].as_u64().unwrap() as usize;
        for budget in [total, 64] {
            let limit = fvid::container::mp4::Limits {
                tracks: budget,
                ..Default::default()
            };
            let mut r =
                fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(bytes(name)), limit)
                    .unwrap();
            assert_eq!(
                r.tracks().len(),
                c["supportedTracks"].as_u64().unwrap() as usize
            );
            assert_eq!(
                r.refused().len(),
                c["unsupportedTracks"].as_u64().unwrap() as usize
            );
            let o = fvid_media::owned_mp4::Mp4Reader::open_indexed(
                Cursor::new(bytes(name)),
                fvid_media::owned_mp4::Limits {
                    tracks: budget,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(o.tracks().len(), r.tracks().len());
            assert_eq!(o.refused().len(), r.refused().len());
            let mut packet = vec![];
            for i in 0..3 {
                let sample = r.tracks()[0].samples.get(i).unwrap();
                assert_eq!((sample.pts, sample.duration), (i as i64, 1));
                r.read_packet(0, i, &mut packet).unwrap();
                assert!(!packet.is_empty());
            }
            let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
                Cursor::new(bytes(name)),
                limit,
                32 << 20,
            )
            .unwrap();
            for _ in 0..3 {
                let f = video.read_frame().unwrap().unwrap();
                let actual = f
                    .picture
                    .y
                    .iter()
                    .chain(&f.picture.cb)
                    .chain(&f.picture.cr)
                    .flat_map(|n| n.to_le_bytes())
                    .collect::<Vec<_>>();
                assert_eq!(actual, bytes("frame.yuv16"), "{name}");
            }
            assert!(video.read_frame().unwrap().is_none());
            for target in [-1, 0, 1, 2, i64::MAX] {
                let landed = video.seek_to_sync(target);
                assert_eq!(landed, target.clamp(0, 2));
                assert!(video.read_frame().unwrap().is_some());
            }
            video.rewind();
            assert_eq!(video.read_frame().unwrap().unwrap().sample_index, 0);
        }
    }
}
#[test]
fn duplicate_track_ids_are_invalid_even_when_the_codec_is_not_decodable() {
    let manifest: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    let mut failures = vec![];
    for c in manifest["bad"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(_) => failures.push(format!("root:{name}:admitted")),
            Err(e) => {
                assert!(matches!(e, fvid::Error::Invalid(_)));
                assert!(e.to_string().contains("duplicate track ID"), "{name}:{e}");
            }
        }
        match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(_) => failures.push(format!("owned:{name}:admitted")),
            Err(e) => {
                assert!(!e.is_unsupported());
                assert!(e.to_string().contains("duplicate track ID"), "{name}:{e}");
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} duplicate-ID admissions:{failures:?}",
        failures.len()
    );
}
