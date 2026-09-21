use fvid::container::mp4::{Limits, Mp4Reader};
use std::io::Cursor;

fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}
fn joined(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn fixture(wide: bool) -> Vec<u8> {
    let mdat = atom(b"mdat", &[1, 2, 3, 4, 5, 6]);
    let mut entry = vec![0u8; 78];
    entry[6..8].copy_from_slice(&1u16.to_be_bytes());
    entry[24..26].copy_from_slice(&16u16.to_be_bytes());
    entry[26..28].copy_from_slice(&16u16.to_be_bytes());
    entry.extend(atom(
        b"avcC",
        &[1, 66, 0, 10, 255, 225, 0, 1, 103, 1, 0, 1, 104],
    ));
    let stsd = atom(b"stsd", &joined(&[words(&[0, 1]), atom(b"avc1", &entry)]));
    let offsets = if wide {
        atom(
            b"co64",
            &joined(&[
                words(&[0, 2]),
                8u64.to_be_bytes().to_vec(),
                11u64.to_be_bytes().to_vec(),
            ]),
        )
    } else {
        atom(b"stco", &words(&[0, 2, 8, 11]))
    };
    let stbl = atom(
        b"stbl",
        &joined(&[
            stsd,
            atom(b"stsz", &words(&[0, 0, 3, 1, 2, 3])),
            atom(b"stts", &words(&[0, 2, 2, 10, 1, 20])),
            atom(b"ctts", &words(&[0x0100_0000, 2, 1, 10, 2, (-5i32) as u32])),
            atom(b"stsc", &words(&[0, 2, 1, 2, 1, 2, 1, 1])),
            offsets,
            atom(b"stss", &words(&[0, 2, 1, 3])),
        ]),
    );
    let dinf = atom(
        b"dinf",
        &atom(
            b"dref",
            &joined(&[words(&[0, 1]), atom(b"url ", &words(&[1]))]),
        ),
    );
    let mdia = atom(
        b"mdia",
        &joined(&[
            atom(b"mdhd", &words(&[0, 0, 0, 1000, 40, 0])),
            atom(
                b"hdlr",
                &joined(&[words(&[0, 0]), b"vide".to_vec(), vec![0; 12]]),
            ),
            atom(b"minf", &joined(&[dinf, stbl])),
        ]),
    );
    let trak = atom(
        b"trak",
        &joined(&[
            atom(b"tkhd", &words(&[0, 0, 0, 7, 0])),
            mdia,
            atom(b"edts", &atom(b"elst", &words(&[0, 1, 40, 5, 0x0001_0000]))),
        ]),
    );
    joined(&[
        mdat,
        atom(
            b"moov",
            &joined(&[atom(b"mvhd", &words(&[0, 0, 0, 1000, 40])), trak]),
        ),
    ])
}

#[test]
fn indexed_packets_timestamps_sync_and_edits() {
    for wide in [false, true] {
        let mut mp4 = Mp4Reader::open(Cursor::new(fixture(wide)), Limits::default()).unwrap();
        assert_eq!(mp4.movie_timescale(), 1000);
        let track = &mp4.tracks()[0];
        assert_eq!(track.id, 7);
        assert_eq!(track.codec, *b"avc1");
        assert_eq!(
            track
                .samples
                .iter()
                .map(|s| (s.offset, s.size, s.dts, s.pts, s.duration, s.sync))
                .collect::<Vec<_>>(),
            vec![
                (8, 1, 0, 10, 10, true),
                (9, 2, 10, 5, 10, false),
                (11, 3, 20, 15, 20, true)
            ]
        );
        assert_eq!(track.edits[0].duration, 40);
        assert_eq!(track.edits[0].media_time, 5);
        let mut packet = Vec::new();
        for (i, expected) in [vec![1], vec![2, 3], vec![4, 5, 6]].iter().enumerate() {
            mp4.read_packet(0, i, &mut packet).unwrap();
            assert_eq!(&packet, expected);
        }
        assert!(mp4.read_packet(0, 3, &mut packet).is_err());
        assert!(mp4.read_packet(1, 0, &mut packet).is_err());
    }
}
fn mutate_word(mut data: Vec<u8>, kind: &[u8; 4], offset: usize, value: u32) -> Vec<u8> {
    let p = data.windows(4).position(|w| w == kind).unwrap() + 4 + offset;
    data[p..p + 4].copy_from_slice(&value.to_be_bytes());
    data
}
#[test]
fn rejects_invalid_tables_offsets_and_external_references() {
    for data in [
        mutate_word(fixture(false), b"stco", 8, 0),
        mutate_word(fixture(false), b"stsz", 8, u32::MAX),
        mutate_word(fixture(false), b"stts", 8, 4),
        mutate_word(fixture(false), b"ctts", 8, 4),
        mutate_word(fixture(false), b"stsc", 8, 2),
        mutate_word(fixture(false), b"stsc", 12, 4),
        mutate_word(fixture(false), b"stss", 12, 1),
        mutate_word(fixture(false), b"mdhd", 12, 0),
        mutate_word(fixture(false), b"url ", 0, 0),
    ] {
        assert!(Mp4Reader::open(Cursor::new(data), Limits::default()).is_err());
    }
}
#[test]
fn every_truncation_is_rejected_and_limits_are_enforced() {
    let data = fixture(false);
    for end in 0..data.len() {
        assert!(
            Mp4Reader::open(Cursor::new(&data[..end]), Limits::default()).is_err(),
            "length {end}"
        );
    }
    for limits in [
        Limits {
            metadata_bytes: 1,
            ..Limits::default()
        },
        Limits {
            samples: 2,
            ..Limits::default()
        },
        Limits {
            tracks: 0,
            ..Limits::default()
        },
    ] {
        assert!(Mp4Reader::open(Cursor::new(&data), limits).is_err());
    }
    let mut mp4 = Mp4Reader::open(
        Cursor::new(data),
        Limits {
            packet_bytes: 2,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(mp4.read_packet(0, 2, &mut Vec::new()).is_err());
}
#[test]
fn extended_box_size_and_fragment_rejection() {
    let mut data = fixture(false);
    let p = data.windows(4).position(|w| w == b"moov").unwrap() - 4;
    let old_size = data.len() - p;
    data.splice(
        p..p + 8,
        joined(&[
            words(&[1]),
            b"moov".to_vec(),
            ((old_size + 8) as u64).to_be_bytes().to_vec(),
        ]),
    );
    Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
    let data = joined(&[fixture(false), atom(b"moof", &[])]);
    assert!(Mp4Reader::open(Cursor::new(data), Limits::default()).is_err());
}

#[test]
fn mutated_metadata_never_panics() {
    let original = fixture(false);
    for at in 14..original.len() {
        for replacement in [0, 1, 127, 255] {
            let mut mutated = original.clone();
            mutated[at] = replacement;
            let _ = Mp4Reader::open(
                Cursor::new(mutated),
                Limits {
                    samples: 32,
                    tracks: 4,
                    metadata_bytes: 4096,
                    packet_bytes: 64,
                },
            );
        }
    }
}
