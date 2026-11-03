use fvid_media::{
    CopyOptions,
    owned_mp4::{Limits, Mp4Reader},
};
use std::{fs::File, io::BufReader, path::Path};
fn check(directory: &str, file: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors")
        .join(directory)
        .join(file);
    let reader = Mp4Reader::open_indexed(
        BufReader::new(File::open(&path).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let info =
        fvid_media::owned_mp4_probe::probe_mp4(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let catalogue =
        fvid_media::owned_probe::probe_with_options(&path, &CopyOptions::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&info).unwrap(),
        serde_json::to_value(&catalogue).unwrap()
    );
    assert_eq!(info.streams.len(), reader.tracks().len());
    for (stream, track) in info.streams.iter().zip(reader.tracks()) {
        let start = if track.edits.is_empty() {
            (0..track.samples.len())
                .filter_map(|i| track.samples.get(i).map(|s| s.pts))
                .min()
        } else {
            Some(0)
        };
        assert_eq!(stream.start, start, "{file}");
        let mut total = Some((0u128, 0u128));
        for i in 0..track.samples.len() {
            total = total.and_then(|(bytes, ticks)| {
                let s = track.samples.get(i)?;
                (s.duration != 0)
                    .then_some((bytes + u128::from(s.size), ticks + u128::from(s.duration)))
            });
        }
        let bitrate = total.and_then(|(bytes, ticks)| {
            if ticks == 0 {
                None
            } else {
                i64::try_from(bytes * 8 * u128::from(track.timescale) / ticks).ok()
            }
        });
        assert_eq!(stream.bit_rate, bitrate, "{file}");
    }
    for max_packets in [Some(1), Some(u64::MAX), None] {
        let options = CopyOptions {
            max_packets,
            max_controlled_bytes: Some(64 << 20),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(
                fvid_media::owned_probe::probe_with_options(&path, &options).unwrap()
            )
            .unwrap(),
            serde_json::to_value(&info).unwrap()
        );
        let error = fvid_media::owned_probe::probe_with_options(
            &path,
            &CopyOptions {
                max_packet_bytes: 1,
                ..options
            },
        )
        .unwrap_err();
        assert!(error.contains("packet exceeds budget"), "{file}: {error}");
    }
}
#[test]
fn compact_and_expanded_probe_metadata_matches_full_sample_oracle_and_budgets() {
    for file in [
        "frames-64.mp4",
        "frames-262144.mp4",
        "control-alias.mp4",
        "control-unused.mp4",
    ] {
        check("mp4-uniform-plan-scan", file);
    }
    for file in [
        "pcm-1000.mp4",
        "pcm-48000.mp4",
        "alac-1000.mp4",
        "alac-48000.mp4",
    ] {
        check("mp4-uniform-seek-width", file);
    }
}

#[test]
fn changing_coded_headers_keep_the_existing_probe_refusal() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp4-uniform-plan-scan/control-endian.mp4");
    assert!(
        fvid_media::owned_mp4_probe::try_probe_mp4(&path)
            .unwrap()
            .is_none()
    );
    assert!(
        fvid_media::owned_mp4_probe::probe_mp4(&path)
            .unwrap_err()
            .contains("cannot yet describe every sample entry")
    );
}
