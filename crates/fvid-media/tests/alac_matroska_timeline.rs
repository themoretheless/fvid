use std::{io::Cursor, path::Path, time::Duration};
#[test]
fn alac_origin_and_real_gaps_survive_full_and_interval_exports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/alac-timeline");
    for (name, gap, delay, tail) in [
        ("offset", 0, 0, 0),
        ("offset-gap", 48, 0, 0),
        ("offset-delay-tail", 0, 17, 13),
        ("offset-discard-last", 0, 0, 128),
    ] {
        let size = if gap == 0 { 128 } else { 96 };
        let mut expected = vec![0; (2400 - delay) * 4];
        for n in 0..size * 4 - tail {
            if n == size * 2 {
                expected.resize(expected.len() + gap * 4, 0);
            }
            expected.extend(((n * 119 % 24001) as f32 / 32768. - 12000. / 32768.).to_le_bytes());
        }
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(49), Duration::from_millis(56))),
        ] {
            let wanted = if interval.is_some() {
                &expected[2352 * 4..2688 * 4]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(pcm.len(), wanted.len(), "{name}");
            assert_eq!(pcm, wanted);
            assert_eq!(stats.sample_frames, wanted.len() as u64 / 4);
        }
    }
}
