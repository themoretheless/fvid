use fvid_codecs::mp3_huffman::Cursor;
#[test]
fn every_codeword_escape_sign_and_alignment_matches_independent_vectors() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-huffman/signed-corpus.bin");
    let bytes = std::fs::read(p).unwrap();
    assert_eq!(&bytes[..8], b"FVMP3H1\0");
    let count = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let mut at = 12;
    for record in 0..count {
        let kind = bytes[at];
        let table = bytes[at + 1];
        let start = usize::from(bytes[at + 2]);
        let bits = usize::from(u16::from_be_bytes(
            bytes[at + 3..at + 5].try_into().unwrap(),
        ));
        let n = usize::from(bytes[at + 5]);
        let expected: [i32; 4] = std::array::from_fn(|i| {
            i32::from(i16::from_le_bytes(
                bytes[at + 6 + i * 2..at + 8 + i * 2].try_into().unwrap(),
            ))
        });
        at += 14;
        let data = &bytes[at..at + n];
        at += n;
        let mut reader = Cursor::new(data, start..start + bits).unwrap();
        let actual = if kind == 0 {
            let p = reader.pair(table).unwrap();
            [p[0], p[1], 0, 0]
        } else {
            reader.quad(table).unwrap()
        };
        assert_eq!(actual, expected, "record {record} table {table}");
        assert_eq!(reader.position(), start + bits);
        assert_eq!(reader.remaining(), 0);
        for limit in 0..bits {
            let mut short = Cursor::new(data, start..start + limit).unwrap();
            let error = if kind == 0 {
                short.pair(table).err()
            } else {
                short.quad(table).err()
            };
            assert!(
                error.is_some(),
                "record {record}: escaped bit boundary {limit}/{bits}"
            );
            assert_eq!(
                short.position(),
                start,
                "failed record {record} consumed bits"
            );
        }
    }
    assert_eq!(at, bytes.len());
    assert!(count > 20000);
}
#[test]
fn normative_small_codewords_and_reserved_tables() {
    let mut pair = Cursor::new(&[0b00010000], 0..5).unwrap();
    assert_eq!(pair.pair(1).unwrap(), [-1, 1]);
    let mut pair = Cursor::new(&[0b01100000], 0..3).unwrap();
    assert_eq!(pair.pair(1).unwrap(), [-1, 0]);
    let mut quad = Cursor::new(&[0b01001010], 0..7).unwrap();
    assert_eq!(quad.quad(1).unwrap(), [-1, 0, 1, -1]);
    let mut zero = Cursor::new(&[], 0..0).unwrap();
    assert_eq!(zero.pair(0).unwrap(), [0, 0]);
    for table in [4, 14, 32, 255] {
        assert!(zero.pair(table).is_err());
        assert_eq!(zero.position(), 0);
    }
    assert!(zero.quad(2).is_err());
    assert!(Cursor::new(&[0], 1..9).is_err());
    assert!(Cursor::new(&[0], 7..6).is_err());
    assert!(zero.read(1).is_err());
    assert_eq!(zero.read(0).unwrap(), 0);
}

#[test]
fn granule_regions_count1_stuffing_and_zero_tables_reconstruct_all_lines() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-huffman/spectrum-cases.bin");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"FVMP3S1\0");
    let count = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let mut at = 12;
    for case in 0..count {
        let word = |offset: usize| {
            usize::from(u16::from_be_bytes(
                bytes[at + offset..at + offset + 2].try_into().unwrap(),
            ))
        };
        let big = word(0) as u16;
        let ends = [word(2), word(4)];
        let tables = bytes[at + 6..at + 9].try_into().unwrap();
        let table = bytes[at + 9];
        let start = usize::from(bytes[at + 10]);
        let bits = word(11);
        let size = word(13);
        let consumed = word(15);
        let big_bits = word(17);
        let nonzero = word(19);
        at += 21;
        let expected: [i32; 576] = std::array::from_fn(|i| {
            i32::from(i16::from_le_bytes(
                bytes[at + i * 2..at + i * 2 + 2].try_into().unwrap(),
            ))
        });
        at += 1152;
        let data = &bytes[at..at + size];
        at += size;
        let mut reader = Cursor::new(data, start..start + bits).unwrap();
        let decoded = reader.spectrum(big, ends, tables, table).unwrap();
        assert_eq!(decoded.values, expected, "case {case}");
        assert_eq!(decoded.nonzero_end, nonzero);
        assert_eq!(decoded.huffman_bits, consumed);
        assert_eq!(reader.position(), start + bits);
        for cut in 0..big_bits {
            let mut short = Cursor::new(data, start..start + cut).unwrap();
            assert!(
                short.spectrum(big, ends, tables, table).is_err(),
                "case {case} accepted incomplete big_values at {cut}"
            );
            assert_eq!(short.position(), start);
        }
    }
    assert_eq!(count, 6);
    assert_eq!(at, bytes.len());
    let mut reader = Cursor::new(&[0xff], 0..8).unwrap();
    for (big, ends, tables, quad) in [
        (289, [0, 0], [0, 0, 0], 0),
        (0, [3, 4], [0, 0, 0], 0),
        (0, [2, 0], [0, 0, 0], 0),
        (0, [0, 578], [0, 0, 0], 0),
        (0, [0, 0], [4, 0, 0], 0),
        (0, [0, 0], [0, 0, 0], 2),
    ] {
        assert!(reader.spectrum(big, ends, tables, quad).is_err());
        assert_eq!(reader.position(), 0);
    }
}
