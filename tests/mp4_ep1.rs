use fvid_media::{
    owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor, owned_mp4::Mp4Reader,
    owned_mp4_ep1::Group,
};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-ep1")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn selected<R>(reader: &Mp4Reader<R>) -> usize
where
    R: std::io::Read + std::io::Seek,
{
    reader
        .tracks()
        .iter()
        .position(|t| {
            t.handler == *b"soun"
                && aac_stream_descriptor(&t.configuration)
                    .unwrap()
                    .depends_on
                    .is_none()
        })
        .unwrap()
}
#[test]
fn real_mp4_ep1_groups_decode_exact_bit_ends_rational_clocks_and_independent_pcm() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 8);
    for case in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default()).unwrap();
        let track = selected(&reader);
        let group = Group::new(reader.tracks(), track).unwrap();
        assert_eq!(group.root_track(), track);
        assert_eq!(
            group.stream_count(),
            case["streams"].as_u64().unwrap() as usize
        );
        let asc = aac_stream_descriptor(&reader.tracks()[track].configuration)
            .unwrap()
            .asc;
        let mut decoder = NativeAacDecoder::new(asc).unwrap();
        let golden = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        let gold: Vec<f32> = golden
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let mut full = Vec::new();
        let mut nonbyte = false;
        let mut nonzero_excluded = false;
        for i in 0..3 {
            let units = group.read_frame(&mut reader, i).unwrap();
            for unit in &units {
                if unit.bit_len % 8 != 0 {
                    nonbyte = true;
                    nonzero_excluded |=
                        unit.data.last().unwrap() & ((1 << (8 - unit.bit_len % 8)) - 1) != 0;
                }
            }
            let mut input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            input.reverse();
            let frame = decoder.decode_esc(&input).unwrap().unwrap();
            assert_eq!(
                frame.pts,
                reader.tracks()[track].samples.get(i).unwrap().pts
            );
            full.extend(frame.samples);
        }
        assert!(
            nonbyte && nonzero_excluded,
            "{} did not exercise ignored transport bits",
            case["name"]
        );
        assert_eq!(full.len(), gold.len());
        let error = full
            .iter()
            .zip(&gold)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} PCM error {error}", case["name"]);
        assert!(group.read_frame(&mut reader, 3).is_err());
        // Stateless container navigation plus real decoder preroll. This is
        // group-read acceptance, not AudioStream/device playback qualification.
        let frame_samples = full.len() / 3;
        for target in [2, 1, 0, 2, 0] {
            decoder.reset();
            let mut suffix = Vec::new();
            for i in 0..3 {
                let units = group.read_frame(&mut reader, i).unwrap();
                let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                let frame = decoder.decode_esc(&input).unwrap().unwrap();
                if i >= target {
                    suffix.extend(frame.samples);
                }
            }
            assert_eq!(
                suffix,
                full[target * frame_samples..],
                "{} read/rewind {target}",
                case["name"]
            );
        }
    }
}
#[test]
fn missing_mistimed_forked_or_ambiguous_es_groups_have_specific_errors() {
    for case in manifest()["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let track = selected(&reader);
        let expected = case["error"].as_str().unwrap();
        match Group::new(reader.tracks(), track) {
            Err(error) => assert!(
                error.to_string().contains(expected),
                "{}: {error}",
                case["mode"]
            ),
            Ok(group) => {
                let index = if case["mode"] == "missing" { 2 } else { 0 };
                let error = match group.read_frame(&mut reader, index) {
                    Ok(_) => panic!("{} accepted", case["mode"]),
                    Err(e) => e,
                };
                assert!(
                    error.to_string().contains(expected),
                    "{}: {error}",
                    case["mode"]
                );
            }
        }
    }
}
