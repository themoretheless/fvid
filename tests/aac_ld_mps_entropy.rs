use fvid_media::owned_aac::{
    aac_ld_mps_entropy::{Kind, Options, decode},
    bits::BitReader,
};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-entropy")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn history(c: &Value) -> Vec<i16> {
    c["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i16)
        .collect()
}
fn options(c: &Value) -> Options {
    Options {
        kind: match c["kind"].as_str().unwrap() {
            "CLD" => Kind::Cld,
            "ICC" => Kind::Icc,
            "CPC" => Kind::Cpc,
            _ => unreachable!(),
        },
        coarse: c["coarse"].as_bool().unwrap(),
        paired: c["paired"].as_bool().unwrap(),
        independent: c["independent"].as_bool().unwrap(),
        set_index: c["set_index"].as_u64().unwrap() as u8,
    }
}
fn shifted(packet: &[u8], length: usize, at: usize) -> Vec<u8> {
    let mut out = vec![0; (at + length + 8).div_ceil(8)];
    for i in 0..length {
        out[(at + i) / 8] |= ((packet[i / 8] >> (7 - i % 8)) & 1) << (7 - (at + i) % 8);
    }
    for i in 0..8 {
        out[(at + length + i) / 8] |= ((0x5au8 >> (7 - i)) & 1) << (7 - (at + length + i) % 8);
    }
    out
}
#[test]
fn all_quantizers_pcm_huffman_contexts_lavs_pairs_and_escape_sequences_match_expected_indices() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 8094);
    for c in m["cases"].as_array().unwrap() {
        let from = c["offset"].as_u64().unwrap() as usize;
        let packet = &blob[from..from + c["bytes"].as_u64().unwrap() as usize];
        let length = c["bits"].as_u64().unwrap() as usize;
        let history = history(c);
        let saved = history.clone();
        let options = options(c);
        for at in 0..8 {
            let bytes = shifted(packet, length, at);
            let mut bits = BitReader::new(&bytes);
            bits.skip(at).unwrap();
            let pair = decode(&mut bits, &history, options)
                .unwrap_or_else(|e| panic!("{} at {at}: {e}", c["name"]));
            let actual = std::iter::once(pair.first)
                .chain(pair.second)
                .collect::<Vec<_>>();
            let expected = c["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_i64().unwrap() as i16)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{} offset {at}", c["name"]);
            assert_eq!(bits.position(), at + length, "{}", c["name"]);
            assert_eq!(bits.read(8).unwrap(), 0x5a);
            assert_eq!(history, saved);
        }
    }
}
#[test]
fn every_bit_truncation_of_each_full_band_coding_context_preserves_cursor_and_history() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let mut checked = 0;
    for c in m["cases"].as_array().unwrap() {
        if c["history"].as_array().unwrap().len() != 23 {
            continue;
        }
        let from = c["offset"].as_u64().unwrap() as usize;
        let packet = &blob[from..from + c["bytes"].as_u64().unwrap() as usize];
        let length = c["bits"].as_u64().unwrap() as usize;
        let h = history(c);
        let saved = h.clone();
        let opts = options(c);
        // Bit limits, rather than zero byte padding, must cause the specific short
        // prefix failure; the sentinel remains present in backing storage.
        for at in [0, 3, 7] {
            let bytes = shifted(packet, length, at);
            for prefix in 0..length {
                let mut bits = BitReader::with_bit_len(&bytes, at + prefix).unwrap();
                bits.skip(at).unwrap();
                assert!(
                    decode(&mut bits, &h, opts).is_err(),
                    "{} prefix {prefix}/{length} at {at}",
                    c["name"]
                );
                assert_eq!(bits.position(), at);
                assert_eq!(h, saved);
            }
        }
        checked += 1;
    }
    assert!(checked > 300);
}
#[test]
fn unused_pcm_words_invalid_indices_and_bad_pair_geometry_are_transactional() {
    let mut opts = Options {
        kind: Kind::Cld,
        coarse: false,
        paired: false,
        independent: true,
        set_index: 0,
    };
    // PCM flag + unused fine-CLD word 31 (only 0..30 exist).
    let bytes = [0xff, 0x5a];
    let mut bits = BitReader::new(&bytes);
    assert!(
        decode(&mut bits, &[0], opts)
            .unwrap_err()
            .to_string()
            .contains("unused LD-MPS PCM")
    );
    assert_eq!(bits.position(), 0);
    for length in [24] {
        let mut bits = BitReader::new(&bytes);
        assert!(decode(&mut bits, &vec![0; length], opts).is_err());
        assert_eq!(bits.position(), 0);
    }
    let mut empty_huffman = BitReader::new(&[0]);
    assert!(
        decode(&mut empty_huffman, &[], opts)
            .unwrap_err()
            .to_string()
            .contains("empty LD-MPS Huffman")
    );
    assert_eq!(empty_huffman.position(), 0);
    for (set, paired) in [(2, false), (1, true)] {
        opts.set_index = set;
        opts.paired = paired;
        let mut bits = BitReader::new(&bytes);
        assert!(decode(&mut bits, &[0], opts).is_err());
        assert_eq!(bits.position(), 0);
    }
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let row = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            c["kind"] == "CLD"
                && c["scheme"] == "1d"
                && c["name"].as_str().unwrap().contains("p0-history")
        })
        .unwrap();
    let start = row["offset"].as_u64().unwrap() as usize;
    let mut bits = BitReader::new(&blob[start..start + row["bytes"].as_u64().unwrap() as usize]);
    assert!(
        decode(&mut bits, &vec![i16::MAX; history(row).len()], options(row))
            .unwrap_err()
            .to_string()
            .contains("parameter history")
    );
    assert_eq!(bits.position(), 0);
    // Neither an independent current-frame pair nor an explicitly frequency-
    // coded dependent vector may inspect irrelevant history values.
    for c in m["cases"].as_array().unwrap().iter().filter(|c| {
        c["scheme"] == "pcm"
            || c["name"]
                .as_str()
                .unwrap()
                .contains("independent-freq-time")
            || c["name"].as_str().unwrap().contains("p0-dependent-freq")
    }) {
        let start = c["offset"].as_u64().unwrap() as usize;
        let mut bits = BitReader::new(&blob[start..start + c["bytes"].as_u64().unwrap() as usize]);
        decode(&mut bits, &vec![i16::MIN; history(c).len()], options(c)).unwrap();
    }
}
#[test]
fn out_of_range_later_vectors_and_the_final_cpc_lsb_roll_back_the_complete_pair() {
    let books: Value =
        serde_json::from_slice(&std::fs::read(root().join("codewords.json")).unwrap()).unwrap();
    let code = |name: &str, value: i64| {
        let r = books[name]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r[2].as_i64() == Some(value))
            .unwrap();
        format!(
            "{:0width$b}",
            r[1].as_u64().unwrap(),
            width = r[0].as_u64().unwrap() as usize
        )
    };
    let base = Options {
        kind: Kind::Cld,
        coarse: false,
        paired: false,
        independent: true,
        set_index: 0,
    };
    let rows = [
        (
            base,
            2,
            format!("00{}{}0", code("CLD_P0", 30), code("CLD_F1", 1)),
        ),
        (
            Options {
                kind: Kind::Icc,
                coarse: true,
                ..base
            },
            1,
            format!("00{}", code("ICC_P0", 7)),
        ),
        (
            Options {
                kind: Kind::Cpc,
                ..base
            },
            1,
            format!("00{}1", code("CPC_P0", 25)),
        ),
        (
            Options {
                coarse: true,
                paired: true,
                ..base
            },
            1,
            format!("0000{}{}", code("CLD_P0", 7), code("CLD_P0", 30)),
        ),
        (
            Options {
                kind: Kind::Cpc,
                paired: true,
                ..base
            },
            1,
            format!("0000{}{}01", code("CPC_P0", 25), code("CPC_P0", 25)),
        ),
    ];
    for (opts, n, wire) in rows {
        let mut bytes = vec![0; wire.len().div_ceil(8)];
        for (i, b) in wire.bytes().enumerate() {
            bytes[i / 8] |= (b - b'0') << (7 - i % 8);
        }
        let mut bits = BitReader::with_bit_len(&bytes, wire.len()).unwrap();
        let h = vec![0; n];
        assert!(
            decode(&mut bits, &h, opts)
                .unwrap_err()
                .to_string()
                .contains("quantization range")
        );
        assert_eq!(bits.position(), 0);
        assert_eq!(h, vec![0; n]);
    }
}

#[test]
fn saved_prefix_facts_are_unambiguous_complete_and_preserve_each_escape_symbol() {
    let books: Value =
        serde_json::from_slice(&std::fs::read(root().join("codewords.json")).unwrap()).unwrap();
    let mut entries = 0;
    for (name, rows) in books.as_object().unwrap() {
        let rows = rows.as_array().unwrap();
        let mut mass = 0u64;
        for (i, row) in rows.iter().enumerate() {
            let len = row[0].as_u64().unwrap();
            let code = row[1].as_u64().unwrap();
            assert!(len > 0 && len <= 32 && code < (1u64 << len));
            mass += 1u64 << (32 - len);
            for other in &rows[..i] {
                let l = other[0].as_u64().unwrap();
                let c = other[1].as_u64().unwrap();
                assert!(!(l <= len && code >> (len - l) == c), "{name}");
                assert!(!(len <= l && c >> (l - len) == code), "{name}");
            }
            entries += 1;
        }
        assert_eq!(mass, 1u64 << 32, "{name}");
    }
    assert_eq!(entries, 1504);
}
