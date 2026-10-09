use std::{io::Cursor, path::Path, time::Duration};
fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("adts-ssr-sbr.json")).unwrap()).unwrap()
}
fn expected(c: &serde_json::Value) -> Vec<u8> {
    let mut pcm = Vec::new();
    let data = std::fs::read(root().join(c["video"]["file"].as_str().unwrap())).unwrap();
    fvid::native_media::decode_mp4_aac_pcm(&data, &mut pcm).unwrap();
    pcm
}
#[test]
fn ssr_adts_sbr_and_core_programs_keep_complete_pcm_and_intervals() {
    for c in cases()["cases"].as_array().unwrap() {
        let expected = expected(c);
        let rate = c["container_rate"].as_u64().unwrap() as u32;
        for name in c["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(name.as_str().unwrap())).unwrap();
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut pcm,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (rate, 1, (expected.len() / 4) as u64)
            );
            assert_eq!(pcm, expected);
            let mut actual = Vec::new();
            fvid::native_media::decode_aac_pcm_interval(
                &data,
                &mut actual,
                &Default::default(),
                None,
            )
            .unwrap();
            assert_eq!(actual, expected);
            let mut interval = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut interval,
                Some((Duration::from_millis(20), Duration::from_millis(200))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                interval,
                expected[(rate as usize / 50) * 4..(rate as usize / 5) * 4]
            );
        }
    }
}
#[test]
fn ssr_adts_remux_negotiates_clock_and_preserves_pcm() {
    use fvid::container::{adts, matroska_write, mp4_write};
    for c in cases()["cases"].as_array().unwrap() {
        let expected = expected(c);
        for name in c["files"].as_array().unwrap() {
            let data = std::fs::read(root().join(name.as_str().unwrap())).unwrap();
            for streaming in [false, true] {
                let mut out = Cursor::new(Vec::new());
                if streaming {
                    mp4_write::write_adts_aac_reader(
                        adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                        &mut out,
                    )
                    .unwrap();
                } else {
                    mp4_write::write_adts_aac(&data, &mut out).unwrap();
                }
                let mut pcm = Vec::new();
                fvid::native_media::decode_mp4_aac_pcm(out.get_ref(), &mut pcm).unwrap();
                assert_eq!(pcm, expected);
            }
            let mut out = Cursor::new(Vec::new());
            matroska_write::write_adts(
                adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                &mut out,
                None,
                None,
            )
            .unwrap();
            let mut pcm = Vec::new();
            fvid::native_media::decode_matroska_aac_pcm_interval(out.get_ref(), &mut pcm, None)
                .unwrap();
            assert_eq!(pcm, expected);
        }
    }
}
