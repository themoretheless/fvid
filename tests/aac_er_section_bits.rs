use fvid_media::owned_aac::{
    aac_ics::IcsInfo,
    aac_synthesis::{WindowSequence, WindowShape},
    bits::BitReader,
};
fn pack(s: &str) -> Vec<u8> {
    let mut bytes = vec![0; s.len().div_ceil(8)];
    for (i, b) in s.bytes().enumerate() {
        bytes[i / 8] |= (b - b'0') << (7 - i % 8);
    }
    bytes
}
fn info(short: bool, bands: u8, groups: Vec<u8>) -> IcsInfo {
    IcsInfo {
        sequence: if short {
            WindowSequence::EightShort
        } else {
            WindowSequence::OnlyLong
        },
        shape: WindowShape::Sine,
        max_sfb: bands,
        group_lengths: groups,
        prediction: None,
    }
}
#[test]
fn resilient_sections_keep_virtual_ids_and_implicit_lengths_in_every_group() {
    for short in [false, true] {
        let groups = if short { vec![1, 3, 4] } else { vec![1] };
        let i = info(short, 18, groups.clone());
        let one = if short { "001" } else { "00001" };
        let row = format!(
            "00001{one}01011{}",
            (16..32)
                .map(|book| format!("{book:05b}"))
                .collect::<String>()
        );
        let wire = row.repeat(groups.len());
        let bytes = pack(&wire);
        let mut bits = BitReader::new(&bytes);
        let expected: Vec<u8> = [1, 11].into_iter().chain(16..32).collect();
        assert_eq!(
            i.read_sections_with_resilience(&mut bits, true).unwrap(),
            vec![expected; groups.len()]
        );
        assert_eq!(bits.position(), wire.len());
        // Every suffix truncation has to fail transactionally. Shift the start
        // so the byte-aligned end exposes exactly the selected bit count.
        for keep in 0..wire.len() {
            let prefix = (8 - keep % 8) % 8;
            let bytes = pack(&format!("{}{}", "0".repeat(prefix), &wire[..keep]));
            let mut bits = BitReader::new(&bytes);
            bits.skip(prefix).unwrap();
            assert!(
                i.read_sections_with_resilience(&mut bits, true).is_err(),
                "short={short} keep={keep}"
            );
            assert_eq!(bits.position(), prefix);
        }
    }
}
#[test]
fn resilient_explicit_sections_escape_and_reject_empty_reserved_overrun() {
    for short in [false, true] {
        let width = if short { 3 } else { 5 };
        let escape = (1 << width) - 1;
        let i = info(short, escape + 2, if short { vec![8] } else { vec![1] });
        let wire = format!("00001{escape:0width$b}{:0width$b}", 2, width = width);
        let bytes = pack(&wire);
        let mut bits = BitReader::new(&bytes);
        assert_eq!(
            i.read_sections_with_resilience(&mut bits, true).unwrap()[0],
            vec![1; usize::from(escape) + 2]
        );
        for wire in [
            format!("01100{:0width$b}", 1, width = width),
            format!("00001{:0width$b}", 0, width = width),
            format!("00001{escape:0width$b}{:0width$b}", 3, width = width),
        ] {
            let bytes = pack(&wire);
            let mut bits = BitReader::new(&bytes);
            assert!(i.read_sections_with_resilience(&mut bits, true).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
