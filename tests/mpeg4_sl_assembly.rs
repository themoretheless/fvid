use fvid_media::owned_sl::{Assembler, Config};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mpeg4-sl-assembly")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn packet<'a>(blob: &'a [u8], row: &Value) -> &'a [u8] {
    let at = row["offset"].as_u64().unwrap() as usize;
    &blob[at..at + row["bytes"].as_u64().unwrap() as usize]
}
fn bits(data: &[u8], len: usize) -> String {
    (0..len)
        .map(|i| {
            if data[i / 8] & (1 << (7 - i % 8)) != 0 {
                '1'
            } else {
                '0'
            }
        })
        .collect()
}
#[test]
fn sl_fragment_sequences_duplicates_clean_eof_and_error_retry_are_transactional() {
    let blob = read("packets.bin");
    for case in manifest()["sequences"].as_array().unwrap() {
        let config = Config::parse(&read(case["config"].as_str().unwrap())).unwrap();
        let mut assembler = Assembler::new(config.clone()).unwrap();
        for _ in 0..2 {
            for step in case["steps"].as_array().unwrap() {
                let retained = assembler.retained_payload_bytes().unwrap();
                let result = if step["finish"] == true {
                    assembler
                        .finish()
                        .map(|u| u.into_iter().collect::<Vec<_>>())
                } else {
                    assembler.push(packet(&blob, &step["packet"])).map(|p| {
                        assert_eq!(p.duplicate_packet, step["duplicate"] == true);
                        if let Some(ocr) = step["ocr"].as_u64() {
                            assert_eq!(p.ocr, Some(ocr));
                        }
                        p.access_units
                    })
                };
                if let Some(expected) = step["error"].as_str() {
                    assert!(
                        result.unwrap_err().to_string().contains(expected),
                        "{}: {expected}",
                        case["name"]
                    );
                    // Continue directly after the error: restoration must not hide mutation.
                    assert_eq!(retained, assembler.retained_payload_bytes().unwrap());
                } else {
                    let actual = result.unwrap();
                    let expected = step["units"].as_array().unwrap();
                    assert_eq!(actual.len(), expected.len(), "{}", case["name"]);
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_eq!(
                            bits(&actual.data, actual.bit_len),
                            expected.as_str().unwrap()
                        );
                    }
                }
            }
            assert!(assembler.finish().unwrap().is_none());
            let first = case["steps"][0]["packet"].clone();
            assert!(
                assembler
                    .push(packet(&blob, &first))
                    .unwrap_err()
                    .to_string()
                    .contains("after end of stream")
            );
            assembler.reset();
        }
        assert!(
            Assembler::with_max_au_bytes(config, Some(1))
                .unwrap()
                .restore(&assembler.checkpoint())
                .is_err()
        );
    }
}
#[test]
fn fragmented_per_es_access_units_decode_to_independent_complete_pcm() {
    use fvid_media::owned_aac::{NativeAacDecoder, aac_ep1::AccessUnit};
    use std::collections::BTreeMap;
    let blob = read("packets.bin");
    let m = manifest();
    assert_eq!(m["native"].as_array().unwrap().len(), 20);
    let source = root().with_file_name("mpeg4-sl");
    for case in m["native"].as_array().unwrap() {
        let asc = std::fs::read(source.join(case["asc"].as_str().unwrap())).unwrap();
        let golden = std::fs::read(source.join(case["golden"].as_str().unwrap())).unwrap();
        let config = Config::parse(&read(case["config"].as_str().unwrap())).unwrap();
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut streams = BTreeMap::<u16, Assembler>::new();
        let mut pcm = Vec::new();
        for row in case["frames"].as_array().unwrap() {
            let mut complete = Vec::new();
            for unit in row["units"].as_array().unwrap() {
                let sid = unit["stream_id"].as_u64().unwrap() as u16;
                let stream = streams
                    .entry(sid)
                    .or_insert_with(|| Assembler::new(config.clone()).unwrap());
                let mut output = Vec::new();
                for fragment in unit["fragments"].as_array().unwrap() {
                    let raw = packet(&blob, fragment);
                    let checkpoint = stream.checkpoint();
                    let actual = stream
                        .push(raw)
                        .unwrap_or_else(|e| panic!("{} stream {sid}: {e}", case["name"]))
                        .access_units;
                    stream.restore(&checkpoint).unwrap();
                    assert_eq!(actual, stream.push(raw).unwrap().access_units);
                    assert!(stream.push(raw).unwrap().duplicate_packet);
                    output.extend(actual);
                }
                assert_eq!(output.len(), 1, "{} stream {sid}", case["name"]);
                let au = output.pop().unwrap();
                assert_eq!(au.bit_len, unit["bits"].as_u64().unwrap() as usize);
                let source_text = unit["source"].as_str().unwrap();
                let expected: Vec<u8> = source_text
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                    .collect();
                assert_eq!(bits(&au.data, au.bit_len), bits(&expected, au.bit_len));
                complete.push((sid, unit["depends_on"].as_u64().map(|n| n as u16), au));
            }
            // Arrival order is not dependency order.
            complete.reverse();
            let input: Vec<_> = complete
                .iter()
                .map(|(sid, parent, au)| AccessUnit {
                    stream_id: *sid,
                    depends_on: *parent,
                    data: &au.data,
                    bit_len: au.bit_len,
                    pts: au.composition_timestamp.unwrap() as i64,
                    duration: row["duration"].as_u64().unwrap(),
                    timescale: config.timestamp_resolution,
                })
                .collect();
            let frame = decoder.decode_esc(&input).unwrap().unwrap();
            assert_eq!(frame.pts, row["pts"].as_i64().unwrap());
            pcm.extend(frame.samples);
        }
        for stream in streams.values_mut() {
            assert!(stream.finish().unwrap().is_none());
        }
        assert_eq!(pcm.len() * 4, golden.len());
        let error = pcm
            .iter()
            .zip(golden.chunks_exact(4))
            .map(|(a, b)| (a - f32::from_le_bytes(b.try_into().unwrap())).abs())
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} PCM {error}", case["name"]);
    }
}

#[test]
fn sl_assembly_preserves_wide_metadata_and_enforces_only_explicit_caller_limit() {
    let base = root().with_file_name("mpeg4-sl");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(base.join("packets.bin")).unwrap();
    for row in manifest["vectors"].as_array().unwrap() {
        let config =
            Config::parse(&std::fs::read(base.join(row["config"].as_str().unwrap())).unwrap())
                .unwrap();
        let raw = packet(&blob, &row["packet"]);
        let mut assembler = Assembler::new(config.clone()).unwrap();
        let p = assembler.push(raw).unwrap();
        assert_eq!(p.ocr, row["value"].as_u64());
        assert_eq!(p.degradation_priority, Some(32767));
        assert_eq!(p.access_units.len(), 1);
        let au = &p.access_units[0];
        assert_eq!(au.bit_len, 9);
        assert_eq!(au.composition_timestamp, row["value"].as_u64());
        assert_eq!(au.sequence, Some(65535));
        if let Some(expected) = row["bitrate"].as_str() {
            let bitrate = p.instant_bitrate.unwrap();
            assert_eq!(bits(&bitrate.data, usize::from(bitrate.bit_len)), expected);
        } else {
            assert!(p.instant_bitrate.is_none());
        }
        assert!(assembler.push(raw).unwrap().duplicate_packet);
        let mut limited = Assembler::with_max_au_bytes(config, Some(1)).unwrap();
        assert!(
            limited
                .push(raw)
                .unwrap_err()
                .to_string()
                .contains("exceeds caller limit")
        );
        assert_eq!(limited.retained_payload_bytes().unwrap(), 0);
        assert!(limited.finish().unwrap().is_none());
    }
}
