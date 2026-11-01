use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-au-origin")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn near(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name} sample count");
    let error = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .map(|(a, b)| {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(a.is_finite());
            (a - b).abs()
        })
        .fold(0.0f32, f32::max);
    assert!(error < 1e-7, "{name}: AU-origin PCM error {error}");
}
#[test]
fn eld_au_origin_matches_complete_cosine_convolution_in_mp4_and_matroska() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for c in cases {
        let gold = read(c["golden"].as_str().unwrap());
        let mut actual = Vec::new();
        let info = fvid::native_media::decode_mp4_aac_pcm(
            &read(c["video"].as_str().unwrap()),
            &mut actual,
        )
        .unwrap();
        assert_eq!(info.sample_frames, c["samples"].as_u64().unwrap());
        near(&actual, &gold, c["name"].as_str().unwrap());
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(actual, mkv, "{} transport", c["name"]);
    }
}
#[test]
fn first_and_final_aus_emit_full_aligned_frames_without_future_input_or_tail_padding() {
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let channels = c["channels"].as_u64().unwrap() as usize;
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let gold = read(c["golden"].as_str().unwrap());
        let mut last = Vec::new();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let end = at + row["bytes"].as_u64().unwrap() as usize;
            let saved = d.checkpoint();
            let output = d.decode(&blob[at..end]).unwrap();
            assert_eq!(
                output.len(),
                n * channels,
                "one full frame immediately per AU"
            );
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(&blob[at..end]).unwrap());
            let bytes: Vec<_> = output.iter().flat_map(|v| v.to_le_bytes()).collect();
            near(
                &bytes,
                &gold[i * n * channels * 4..(i + 1) * n * channels * 4],
                c["name"].as_str().unwrap(),
            );
            last = output;
        }
        if c["recipe"] == "last" {
            assert!(
                last[last.len() - n / 4 * channels..]
                    .iter()
                    .any(|v| v.abs() > 1e-6),
                "the final quarter contains known signal,not fabricated padding"
            );
        }
        d.reset();
        let row = &c["frames"][0];
        let at = row["offset"].as_u64().unwrap() as usize;
        let output = d
            .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
            .unwrap();
        let bytes: Vec<_> = output.iter().flat_map(|v| v.to_le_bytes()).collect();
        near(&bytes, &gold[..n * channels * 4], "reset AU origin");
    }
}
