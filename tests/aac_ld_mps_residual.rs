//! Residual extension syntax and spectra only. The movie test below is an
//! explicit native refusal regression, not residual playback acceptance.
use fvid_media::owned_aac::{
    aac_ld_mps_config::{Config, ExtensionValue},
    aac_ld_mps_residual as residual,
    aac_synthesis::{WindowSequence, WindowShape},
    bits::BitReader,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-residual")
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn configuration(c: &Value) -> (Config, ExtensionValue) {
    let mut cfg = Config::parse(
        &std::fs::read(
            root()
                .parent()
                .unwrap()
                .join(format!("aac-eld-mps-core/tree{}.ssc", n(&c["tree"]))),
        )
        .unwrap(),
    )
    .unwrap();
    cfg.frame_slots = n(&c["slots"]) as u8;
    let e = ExtensionValue::Residual {
        rate: n(&c["rate"]) as u32,
        frames_code: n(&c["frames_code"]) as u8,
        bands: c["bands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().map(|n| n as u8))
            .collect(),
    };
    (cfg, e)
}
fn check(s: &residual::Spectrum, expected: &Value) -> usize {
    assert_eq!(
        s.sequence,
        [
            WindowSequence::OnlyLong,
            WindowSequence::LongStart,
            WindowSequence::EightShort,
            WindowSequence::LongStop
        ][n(&expected["sequence"])]
    );
    assert_eq!(
        s.shape,
        if n(&expected["shape"]) == 0 {
            WindowShape::Sine
        } else {
            WindowShape::Kbd
        }
    );
    let mut spectrum = vec![0f32; 1024];
    for pair in expected["sparse"].as_array().unwrap() {
        spectrum[n(&pair[0])] = pair[1].as_i64().unwrap() as f32;
    }
    assert_eq!(s.coefficients, spectrum);
    1024
}
#[test]
fn residual_icc_and_individual_channel_spectra_match_analytical_stimuli() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(root().join("payloads.bin")).unwrap();
    let mut coefficients = 0;
    let mut extras = 0;
    let mut ttt = 0;
    for c in m["cases"].as_array().unwrap() {
        let (cfg, ext) = configuration(c);
        let raw = &blob[n(&c["offset"])..n(&c["offset"]) + n(&c["bytes"])];
        let mut bits = BitReader::new(raw);
        let frame = residual::read(&mut bits, &cfg, &ext, 2)
            .unwrap_or_else(|e| panic!("tree{} rate{} seq{}: {e}", c["tree"], c["rate"], c["seq"]));
        assert_eq!(bits.position(), n(&c["bits"]));
        assert_eq!(bits.read(8).unwrap(), 0xa5);
        assert_eq!(frame.boxes.len(), c["boxes"].as_array().unwrap().len());
        for (row, (actual, expected)) in frame
            .boxes
            .iter()
            .zip(c["boxes"].as_array().unwrap())
            .enumerate()
        {
            if expected.is_null() {
                assert!(actual.is_none());
                continue;
            }
            let actual = actual.as_ref().unwrap();
            let icc: Vec<Option<Vec<u8>>> = expected["icc"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_array().map(|r| r.iter().map(|v| n(v) as u8).collect()))
                .collect();
            assert_eq!(actual.icc_differences, icc);
            if row >= cfg.tree.ott_lfe.len() {
                assert!(icc.is_empty());
                ttt += 1;
            }
            assert_eq!(
                actual.blocks.len(),
                expected["blocks"].as_array().unwrap().len()
            );
            for (a, e) in actual
                .blocks
                .iter()
                .zip(expected["blocks"].as_array().unwrap())
            {
                coefficients += check(&a.first, &e["first"]);
                if e["extra"].is_null() {
                    assert!(a.extra.is_none());
                } else {
                    coefficients += check(a.extra.as_ref().unwrap(), &e["extra"]);
                    extras += 1;
                }
            }
        }
        assert!(frame.retained_payload_bytes().unwrap() > 0);
        for end in [0, n(&c["bits"]) / 2, n(&c["bits"]) - 1] {
            let mut truncated = BitReader::with_bit_len(raw, end).unwrap();
            assert!(residual::read(&mut truncated, &cfg, &ext, 2).is_err());
            assert_eq!(truncated.position(), 0);
        }
        let mut retry = BitReader::new(raw);
        assert_eq!(residual::read(&mut retry, &cfg, &ext, 2).unwrap(), frame);
    }
    assert!(extras > 0 && ttt > 0);
    eprintln!(
        "Residual analytical coefficients {coefficients}, extra short spectra {extras}, active TTT cases {ttt}"
    );
}
#[test]
fn mismatched_extra_short_ics_is_a_specific_atomic_failure() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let c = &m["extra_mismatch"];
    let blob = std::fs::read(root().join("payloads.bin")).unwrap();
    let (cfg, e) = configuration(c);
    let mut b = BitReader::with_bit_len(
        &blob[n(&c["offset"])..n(&c["offset"]) + n(&c["bytes"])],
        n(&c["bits"]),
    )
    .unwrap();
    assert!(
        residual::read(&mut b, &cfg, &e, 2)
            .unwrap_err()
            .to_string()
            .contains("extra short block has different window sequence")
    );
    assert_eq!(b.position(), 0);
}
#[test]
fn native_residual_movie_accepts_owned_reconstruction_instead_of_refusing() {
    let ssc = std::fs::read(root().join("native.ssc")).unwrap();
    let cfg = Config::parse(&ssc).unwrap();
    assert!(cfg.extensions.iter().any(|e|matches!(&e.value,ExtensionValue::Residual{bands,..}if bands.iter().any(|b|b.unwrap_or(0)>0))));
    // Prove that every spatial frame and its compressed residual payload are
    // syntactically valid; the formerly refused movie now produces owned PCM.
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let extension = &cfg.extensions[0].value;
    let mut frames = fvid_media::owned_aac::aac_ld_mps_frame::Decoder::new(&cfg).unwrap();
    for row in manifest["native"]["frames"].as_array().unwrap() {
        let wire = row["spatial"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        let mut reader = BitReader::new(&wire);
        let frame = frames.decode(&mut reader).unwrap();
        assert_eq!(reader.remaining(), 0);
        let mut residual_bits = BitReader::new(&frame.extensions[0].payload);
        let spectrum =
            residual::read(&mut residual_bits, &cfg, extension, frame.slots.len()).unwrap();
        assert!(
            spectrum.boxes[0].as_ref().unwrap().blocks[0]
                .first
                .coefficients
                .iter()
                .any(|v| *v != 0.)
        );
        assert!(residual_bits.remaining() < 8);
        while residual_bits.remaining() > 0 {
            assert!(!residual_bits.bit().unwrap());
        }
    }
    let movie = std::fs::read(root().join("native-residual-refusal.mp4")).unwrap();
    let mut pcm = Vec::new();
    let stats = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm).unwrap();
    assert_eq!(stats.sample_frames, 12288);
    assert_eq!(stats.decoded_frames, 12);
    assert_eq!(pcm.len(), 12288 * 2 * 4);
    assert!(pcm.iter().any(|&v| v != 0));
}
