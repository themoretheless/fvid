const CORPUS: &str = "aac-eld-sbr-eighth";
const CASES: usize = 532;
const EXPORTS: usize = 258;
include!("support/aac_eld_downscaled.rs");

#[cfg(feature = "player")]
#[test]
fn eighth_core_without_sbr_matches_independent_pcm_and_exact_navigation() {
    use fvid::audio::AudioStream;
    use fvid_media::owned_aac::NativeAacDecoder;
    let root = root().join("core");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(root.join("packets.bin")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 2);
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut native = NativeAacDecoder::new_with_output_rate(&asc, 3000).unwrap();
        let mut actual = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            actual.extend(
                native
                    .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                    .unwrap(),
            );
        }
        let gold = std::fs::read(root.join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(actual.len() * 4, gold.len());
        assert!(
            actual
                .iter()
                .zip(gold.chunks_exact(4))
                .all(|(a, b)| (*a - f32::from_le_bytes(b.try_into().unwrap())).abs() < 1e-7)
        );
        let mp4 = std::fs::read(root.join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root.join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        let mut other = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut other, None).unwrap();
        assert!(full == other);
        assert_eq!(
            full,
            actual
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<_>>()
        );
        let mut sources: Vec<Box<dyn AudioStream>> = vec![
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(mp4),
                    Default::default(),
                )
                .unwrap(),
            ),
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(mkv),
                    Default::default(),
                )
                .unwrap(),
            ),
        ];
        for source in &mut sources {
            assert_eq!(source.sample_rate(), 3000);
            let mut decoder = source.make_decoder().unwrap();
            assert!(playback::play(source.as_mut(), &mut decoder) == full);
            let width = usize::from(source.channels()) * 4;
            let n = c["out_n"].as_u64().unwrap() as usize;
            for sample in [0, 13, n - 1, n, n + 1, full.len() / width, 0] {
                let ticks = (sample as u128 * u128::from(source.timescale()) / 3000) as i64;
                let landed = source.seek_to(ticks);
                source.reset_decoder(&mut decoder).unwrap();
                let begin = (u128::try_from(landed).unwrap() * 3000)
                    .div_ceil(u128::from(source.timescale())) as usize
                    * width;
                assert!(
                    playback::play(source.as_mut(), &mut decoder) == full[begin.min(full.len())..]
                );
            }
            source.rewind();
            source.reset_decoder(&mut decoder).unwrap();
            assert!(playback::play(source.as_mut(), &mut decoder) == full);
        }
    }
}
