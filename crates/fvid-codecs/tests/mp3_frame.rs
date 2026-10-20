use fvid_codecs::mp3_frame::{Frame, Header, Reservoir, Version};
fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-side")
}
fn frame(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(format!("{name}.frame"))).unwrap()
}
fn observed(f: &Frame<'_>) -> Vec<u32> {
    let h = f.header;
    let mut v = vec![
        match h.version {
            Version::Mpeg1 => 3,
            Version::Mpeg2 => 2,
            Version::Mpeg25 => 0,
        },
        h.sample_rate,
        u32::from(h.channels),
        h.frame_bytes as u32,
        u32::from(h.crc),
        u32::from(h.free),
        u32::from(h.mode),
        u32::from(h.mode_extension),
        u32::from(f.side.main_data_begin),
    ];
    for ch in f.side.scfsi {
        v.extend(ch.map(u32::from));
    }
    v.push(h.granules() as u32);
    for group in f.side.granules.iter().take(h.granules()) {
        for g in group.iter().take(usize::from(h.channels)) {
            v.extend([
                u32::from(g.part2_3_bits),
                u32::from(g.big_values),
                u32::from(g.global_gain),
                u32::from(g.scalefac_compress),
                g.window as u32,
                u32::from(g.mixed),
            ]);
            v.extend(g.tables.map(u32::from));
            v.extend(g.subblock_gain.map(u32::from));
            let (a, b) = g.regions.unwrap_or((255, 255));
            v.extend([
                u32::from(a),
                u32::from(b),
                g.preflag.map(u32::from).unwrap_or(255),
                u32::from(g.scalefac_scale),
                u32::from(g.count1_table),
            ]);
        }
    }
    v
}
#[test]
fn authored_side_information_preserves_every_field_without_claiming_pcm_decode() {
    let mut count = 0;
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|v| v != "frame") {
            continue;
        }
        let raw = std::fs::read(&p).unwrap();
        let f = Frame::parse(&raw).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let expected: Vec<u32> = std::fs::read_to_string(p.with_extension("expected"))
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(observed(&f), expected, "{}", p.display());
        count += 1;
    }
    assert_eq!(count, 168);
}
#[test]
fn correct_crc_does_not_hide_reserved_or_inconsistent_side_information() {
    for (name, expected) in [
        ("big-values", "big_values exceeds"),
        ("huffman-table", "reserved MP3 Huffman table"),
        ("region-counts", "region counts exceed"),
        ("switched-normal", "switched window cannot"),
    ] {
        let raw = std::fs::read(dir().join(format!("{name}.invalid"))).unwrap();
        let e = Frame::parse(&raw).err().unwrap().to_string();
        assert!(e.contains(expected), "{name}: {e}");
        assert!(!e.contains("CRC"), "unrelated refusal: {e}");
    }
    let raw = frame("v3-c2-w0-m0-crc1-p0");
    for end in 0..raw.len() {
        assert!(Frame::parse(&raw[..end]).is_err());
    }
    let mut bad = raw.clone();
    bad[6] ^= 1;
    assert!(
        Frame::parse(&bad)
            .err()
            .unwrap()
            .to_string()
            .contains("CRC mismatch")
    );
}
#[test]
fn reservoir_stages_bounded_bit_ranges_and_commits_only_once() {
    let a = frame("v3-c2-w0-m0-crc0-p0");
    let b = frame("v3-c2-w2-m0-crc0-p0");
    let first = Frame::parse(&a).unwrap();
    let second = Frame::parse(&b).unwrap();
    let mut reservoir = Reservoir::default();
    assert!(reservoir.prepare(&second).is_err());
    let pending = reservoir.prepare(&first).unwrap();
    let bits: usize = first
        .side
        .granules
        .iter()
        .flatten()
        .map(|g| usize::from(g.part2_3_bits))
        .sum();
    assert_eq!(pending.bytes, &first.main_data[..bits.div_ceil(8)]);
    assert_eq!(pending.ranges.len(), 4);
    assert_eq!(pending.ranges.last().unwrap().end, bits);
    reservoir.commit(pending).unwrap();
    let stage = reservoir.prepare(&second).unwrap();
    let stale = reservoir.prepare(&second).unwrap();
    let n = stage.bytes.len();
    let mut expected = first.main_data[first.main_data.len() - 8..].to_vec();
    expected.extend_from_slice(second.main_data);
    assert_eq!(stage.bytes, &expected[..n]);
    reservoir.commit(stage).unwrap();
    assert!(
        reservoir
            .commit(stale)
            .err()
            .unwrap()
            .to_string()
            .contains("stale")
    );
    let mut third = Frame::parse(&a).unwrap();
    third.side.main_data_begin = 511;
    let stage = reservoir.prepare(&third).unwrap();
    let mut full = first.main_data.to_vec();
    full.extend_from_slice(second.main_data);
    let start = full.len() - 511;
    assert_eq!(stage.bytes, &full[start..start + stage.bytes.len()]);
    let stale = reservoir.prepare(&first).unwrap();
    reservoir.reset();
    assert!(reservoir.commit(stale).is_err());
    assert!(reservoir.prepare(&second).is_err());
}
#[test]
fn saved_synthetic_mp3_frames_parse_and_stage_their_real_reservoir() {
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mp3/tone.mp3");
    let data = std::fs::read(p).unwrap();
    let mut at = 0;
    if data.starts_with(b"ID3") {
        at = 10
            + data[6..10]
                .iter()
                .fold(0usize, |a, &b| (a << 7) | usize::from(b & 127));
    }
    let mut reservoir = Reservoir::default();
    let mut count = 0;
    while at + 4 <= data.len() {
        let h = Header::parse(&data[at..]).unwrap();
        let f = Frame::parse(&data[at..at + h.frame_bytes]).unwrap();
        let staged = reservoir
            .prepare(&f)
            .unwrap_or_else(|e| panic!("frame {count}: {e}"));
        reservoir.commit(staged).unwrap();
        at += h.frame_bytes;
        count += 1;
    }
    assert_eq!(at, data.len());
    assert!(count >= 10);
}

#[test]
fn bounded_free_frame_checks_side_extent_and_reservoir_does_not_copy_ancillary_bulk() {
    let mut raw = frame("v3-c2-w0-m0-crc0-p0");
    raw[2] &= 15;
    let parsed = Frame::parse(&raw).unwrap();
    assert!(parsed.header.free);
    let error = Frame::parse(&raw[..35]).err().unwrap().to_string();
    assert!(error.contains("truncated MP3 side information"), "{error}");
    let bulk = vec![0x5a; 1024 * 1024];
    let mut f = Frame::parse(&raw).unwrap();
    f.main_data = &bulk;
    let mut reservoir = Reservoir::default();
    let stage = reservoir.prepare(&f).unwrap();
    assert!(stage.bytes.len() <= 2048);
    assert!(stage.bytes.iter().all(|&b| b == 0x5a));
    reservoir.commit(stage).unwrap();
    f.side.main_data_begin = 511;
    assert!(reservoir.prepare(&f).is_ok());
    f.side.main_data_begin = 512;
    assert!(reservoir.prepare(&f).is_err());
    f.side.main_data_begin = 0;
    f.main_data = &[];
    assert!(
        reservoir
            .prepare(&f)
            .err()
            .unwrap()
            .to_string()
            .contains("exceed available")
    );
}
