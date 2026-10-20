use fvid_codecs::{
    mp3_frame::{Granule, Version},
    mp3_huffman::Cursor,
    mp3_hybrid::Window,
    mp3_scalefactors::{self, ScaleFactors},
};

fn window(n: u8) -> Window {
    match n {
        0 => Window::Normal,
        1 => Window::Start,
        2 => Window::Short,
        3 => Window::Stop,
        _ => panic!("invalid fixture window"),
    }
}
fn previous() -> ScaleFactors {
    ScaleFactors {
        long: std::array::from_fn(|i| {
            if i < 21 {
                ((i * 7 + 3) % (1 << if i < 11 { 4 } else { 3 })) as u8
            } else {
                0
            }
        }),
        long_bits: std::array::from_fn(|i| {
            if i < 11 {
                4
            } else if i < 21 {
                3
            } else {
                0
            }
        }),
        short: [[0; 3]; 13],
        short_bits: [[0; 3]; 13],
        preflag: false,
        intensity_scale: false,
        window: Window::Normal,
    }
}
fn flattened(s: &ScaleFactors) -> Vec<u8> {
    let mut v = vec![
        s.preflag as u8,
        s.intensity_scale as u8,
        match s.window {
            Window::Normal => 0,
            Window::Start => 1,
            Window::Short => 2,
            Window::Stop => 3,
        },
    ];
    v.extend(s.long);
    v.extend(s.short.iter().flatten());
    v.extend(s.long_bits);
    v.extend(s.short_bits.iter().flatten());
    v
}

#[test]
fn all_compress_layout_reuse_and_alignment_vectors_and_every_truncation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-scalefactors/fields.bin");
    let data = std::fs::read(path).unwrap();
    assert_eq!(&data[..8], b"FVMP3F1\0");
    let count = u32::from_be_bytes(data[8..12].try_into().unwrap());
    assert_eq!(count, 16960);
    let mut at = 12;
    for record in 0..count {
        let h = &data[at..at + 14];
        at += 14;
        let version = match h[0] {
            3 => Version::Mpeg1,
            2 => Version::Mpeg2,
            0 => Version::Mpeg25,
            _ => panic!(),
        };
        let g = Granule {
            window: window(h[1]),
            mixed: h[2] != 0,
            scalefac_compress: u16::from_be_bytes(h[3..5].try_into().unwrap()),
            preflag: (version == Version::Mpeg1).then_some(h[9] != 0),
            ..Granule::default()
        };
        let scfsi = std::array::from_fn(|i| h[6] & (1 << i) != 0);
        let start = usize::from(h[8]);
        let bits = usize::from(u16::from_be_bytes(h[10..12].try_into().unwrap()));
        let length = usize::from(u16::from_be_bytes(h[12..14].try_into().unwrap()));
        let expected = &data[at..at + 125];
        at += 125;
        let payload = &data[at..at + length];
        at += length;
        let prev = previous();
        let prev = (version == Version::Mpeg1 && h[5] == 1).then_some(&prev);
        let mut cursor = Cursor::new(payload, start..start + bits).unwrap();
        let actual = mp3_scalefactors::read(&mut cursor, version, &g, h[5], scfsi, prev, h[7] != 0)
            .unwrap_or_else(|e| panic!("record {record}: {e}"));
        assert_eq!(flattened(&actual), expected, "record {record}");
        assert_eq!(cursor.position(), start + bits, "record {record}");
        for limit in 0..bits {
            let mut cursor = Cursor::new(payload, start..start + limit).unwrap();
            assert!(
                mp3_scalefactors::read(&mut cursor, version, &g, h[5], scfsi, prev, h[7] != 0)
                    .is_err(),
                "record {record} truncated to {limit}/{bits}"
            );
            assert_eq!(cursor.position(), start, "failed read consumed bits");
        }
    }
    assert_eq!(at, data.len());
}

#[test]
fn semantic_errors_preserve_cursor_and_zero_width_intensity_is_zero() {
    let data = [0xff; 128];
    let previous = previous();
    let mut short_previous = previous.clone();
    short_previous.window = Window::Short;
    let base = Granule::default();
    let reuse = [true, false, false, false];
    for (version, g, index, flags, prev) in [
        (Version::Mpeg1, base.clone(), 1, reuse, None),
        (
            Version::Mpeg1,
            base.clone(),
            1,
            reuse,
            Some(&short_previous),
        ),
        (
            Version::Mpeg1,
            Granule {
                window: Window::Short,
                ..base.clone()
            },
            0,
            reuse,
            None,
        ),
        (Version::Mpeg2, base.clone(), 0, reuse, None),
        (
            Version::Mpeg1,
            Granule {
                scalefac_compress: 16,
                ..base.clone()
            },
            0,
            [false; 4],
            None,
        ),
        (
            Version::Mpeg25,
            Granule {
                scalefac_compress: 512,
                ..base.clone()
            },
            0,
            [false; 4],
            None,
        ),
        (Version::Mpeg1, base.clone(), 2, [false; 4], None),
        (Version::Mpeg2, base.clone(), 1, [false; 4], None),
    ] {
        let mut cursor = Cursor::new(&data, 3..1000).unwrap();
        assert!(
            mp3_scalefactors::read(&mut cursor, version, &g, index, flags, prev, false).is_err()
        );
        assert_eq!(cursor.position(), 3);
    }
    for version in [Version::Mpeg2, Version::Mpeg25] {
        let mut cursor = Cursor::new(&data, 7..7).unwrap();
        let s =
            mp3_scalefactors::read(&mut cursor, version, &base, 0, [false; 4], None, true).unwrap();
        assert_eq!(s.long, [0; 22]);
        assert_eq!(s.long_bits, [0; 22]);
        assert!(!s.preflag && !s.intensity_scale);
        assert_eq!(cursor.position(), 7);
    }
}

#[test]
fn saved_synthetic_compressed_frames_read_scalefactors_with_real_reservoir_and_scfsi() {
    use fvid_codecs::mp3_frame::{Frame, Header, Reservoir};
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mp3/tone.mp3");
    let data = std::fs::read(path).unwrap();
    let mut at = if data.starts_with(b"ID3") {
        10 + data[6..10]
            .iter()
            .fold(0usize, |n, &b| (n << 7) | usize::from(b & 127))
    } else {
        0
    };
    let mut reservoir = Reservoir::default();
    let mut frames = 0;
    let mut reused = 0;
    while at + 4 <= data.len() {
        let header = Header::parse(&data[at..]).unwrap();
        let frame = Frame::parse(&data[at..at + header.frame_bytes]).unwrap();
        let staged = reservoir.prepare(&frame).unwrap();
        let mut prev: [Option<ScaleFactors>; 2] = [None, None];
        let mut range = 0;
        for granule in 0..header.granules() {
            for ch in 0..usize::from(header.channels) {
                let bounds = staged.ranges[range].clone();
                range += 1;
                let mut cursor = Cursor::new(&staged.bytes, bounds.clone()).unwrap();
                let scalefactors = mp3_scalefactors::read(
                    &mut cursor,
                    header.version,
                    &frame.side.granules[granule][ch],
                    granule as u8,
                    frame.side.scfsi[ch],
                    prev[ch].as_ref(),
                    header.mode == 1 && header.mode_extension & 1 != 0 && ch == 1,
                )
                .unwrap();
                assert!(cursor.position() <= bounds.end);
                if granule == 1 {
                    reused += frame.side.scfsi[ch].iter().filter(|&&v| v).count();
                }
                prev[ch] = Some(scalefactors);
            }
        }
        reservoir.commit(staged).unwrap();
        at += header.frame_bytes;
        frames += 1;
    }
    assert_eq!(at, data.len());
    assert!(frames >= 10);
    assert!(reused > 0, "fixture must exercise transmitted SCFSI reuse");
}
