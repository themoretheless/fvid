use fvid_media::owned_aac::{
    NativeAacDecoder, aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_pcm,
    bits::BitReader,
};
use serde_json::Value;
use std::io::Cursor;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> Value {
    serde_json::from_slice(
        &std::fs::read(root().join("aac-ld-nearest-rate/manifest.json")).unwrap(),
    )
    .unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join("aac-ld-nearest-rate").join(name)).unwrap()
}
fn near(actual: &[u8], expected: &[u8], tolerance: f32) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(
            a.is_finite() && (a - b).abs() < tolerance,
            "sample{i}: {a} vs {b}"
        );
    }
}
fn gold<'a>(c: &Value, data: &'a [u8]) -> &'a [u8] {
    let at = n(&c["core_reference_offset"]);
    &data[at..at + n(&c["core_reference_bytes"])]
}

#[test]
fn nominal_controls_accept_the_same_nonzero_high_band_and_ltp_payloads() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("aac-ld-layout.json")).unwrap()).unwrap();
    let blob = std::fs::read(root().join("aac-ld-layout-packets.bin")).unwrap();
    let golden = std::fs::read(root().join("aac-ld-layout-reference.f32le")).unwrap();
    let mut controls = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| matches!(n(&c["rate"]), 24000 | 48000))
    {
        let mut decoder = NativeAacDecoder::new(&hex(&c["asc"])).unwrap();
        let mut pcm = Vec::new();
        for f in c["frames"].as_array().unwrap() {
            let at = n(&f["offset"]);
            for v in decoder.decode(&blob[at..at + n(&f["bytes"])]).unwrap() {
                pcm.extend(v.to_le_bytes());
            }
        }
        let at = n(&c["reference_offset"]);
        near(&pcm, &golden[at..at + n(&c["reference_bytes"])], 1e-10);
        assert!(pcm.iter().any(|&v| v != 0));
        controls += 1;
    }
    assert_eq!(controls, 40);
}

#[test]
fn nearest_rate_core_movies_preserve_the_frozen_cosine_reference() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 360);
    let golden = std::fs::read(root().join("aac-ld-layout-reference.f32le")).unwrap();
    let mut count = 0;
    for c in cases.iter().filter(|c| c["spatial"] == false) {
        let mut pcm = Vec::new();
        let stats =
            fvid::native_media::decode_mp4_aac_pcm(&read(c["video"].as_str().unwrap()), &mut pcm)
                .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(
            (
                stats.sample_rate as usize,
                usize::from(stats.channels),
                stats.sample_frames as usize
            ),
            (n(&c["rate"]), n(&c["channels"]), n(&c["samples"]))
        );
        near(&pcm, gold(c, &golden), 1e-10);
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_reader(
            fvid::container::webm::WebmReader::open(
                Cursor::new(read(c["file"].as_str().unwrap())),
                Default::default(),
            )
            .unwrap(),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(mkv, pcm, "{} Matroska", c["name"]);
        count += 1;
    }
    assert_eq!(count, 200);
}

#[test]
fn nearest_rate_spatial_movies_match_the_frozen_core_composition() {
    let m = manifest();
    let golden = std::fs::read(root().join("aac-ld-layout-reference.f32le")).unwrap();
    let mut count = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["spatial"] == true)
    {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let mut spatial = aac_ld_mps_pcm::Decoder::new(&cfg, Mode::Complex).unwrap();
        let input: Vec<_> = gold(c, &golden)
            .chunks_exact(4)
            .map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap())))
            .collect();
        let inputs = n(&c["input_channels"]);
        let samples = n(&c["n"]) * n(&c["group"]);
        let width = n(&c["channels"]);
        let mut expected = Vec::new();
        for (i, packet) in c["reference"].as_array().unwrap().iter().enumerate() {
            let planar: Vec<Vec<_>> = (0..inputs)
                .map(|ch| {
                    (0..samples)
                        .map(|t| input[((i * samples + t) * inputs) + ch])
                        .collect()
                })
                .collect();
            let frame = spatial
                .decode(
                    &mut BitReader::new(&hex(packet)),
                    &planar,
                    &vec![None; cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes)],
                )
                .unwrap();
            for row in frame.samples.chunks_exact(width) {
                for ch in 0..width {
                    let at = if matches!(n(&c["tree"]), 4 | 6) {
                        [0, 1, 2, 3, 6, 7, 4, 5][ch]
                    } else {
                        ch
                    };
                    expected.extend((row[at] as f32).to_le_bytes());
                }
            }
        }
        let mut actual = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(
            &read(c["video"].as_str().unwrap()),
            &mut actual,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(
            (
                stats.sample_rate as usize,
                usize::from(stats.channels),
                stats.sample_frames as usize
            ),
            (n(&c["rate"]), width, n(&c["samples"]))
        );
        near(&actual, &expected, 1e-7);
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_reader(
            fvid::container::webm::WebmReader::open(
                Cursor::new(read(c["file"].as_str().unwrap())),
                Default::default(),
            )
            .unwrap(),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(mkv, actual, "{} Matroska", c["name"]);
        count += 1;
    }
    assert_eq!(count, 160);
}
