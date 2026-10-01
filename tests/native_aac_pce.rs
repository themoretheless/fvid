use fvid::codec::{
    aac_pce::{Position, ProgramConfig},
    bits::BitReader,
};
#[test]
fn parses_independently_encoded_eight_channel_pce_and_all_truncations() {
    let file = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    let size = ((file[3] as usize & 3) << 11) | ((file[4] as usize) << 3) | (file[5] as usize >> 5);
    let packet = &file[7..size];
    let mut bits = BitReader::new(packet);
    assert_eq!(bits.read(3).unwrap(), 5);
    let pce = ProgramConfig::read(&mut bits, 0).unwrap();
    assert_eq!(
        (pce.object_type, pce.sample_rate, pce.channels()),
        (2, 48000, 8)
    );
    assert_eq!(
        pce.elements
            .iter()
            .map(|e| (e.position, e.pair, e.tag))
            .collect::<Vec<_>>(),
        vec![
            (Position::Front, false, 0),
            (Position::Front, true, 0),
            (Position::Front, true, 1),
            (Position::Back, true, 2),
            (Position::Lfe, false, 0)
        ]
    );
    assert!(pce.coupling.is_empty());
    assert!(pce.associated_data.is_empty());
    let end = bits.position();
    assert_eq!(end % 8, 0);
    for length in 1..end / 8 {
        let mut truncated = BitReader::new(&packet[..length]);
        truncated.read(3).unwrap();
        assert!(
            ProgramConfig::read(&mut truncated, 0).is_err(),
            "length {length}"
        );
        assert_eq!(truncated.position(), 3);
    }
}

#[test]
fn asc_preserves_program_without_claiming_standard_layout_decode() {
    use fvid::{codec::config::AacConfig, container::mp4::Mp4Reader};
    let reader = Mp4Reader::open(
        std::io::Cursor::new(include_bytes!("fixtures/audio/aac-pce-wide8.m4a")),
        Default::default(),
    )
    .unwrap();
    let track = &reader.tracks()[0];
    let asc = fvid::codec::config::aac_specific_config(&track.configuration).unwrap();
    let (config, program) = AacConfig::parse_with_program(asc).unwrap();
    assert_eq!((config.sample_rate, config.channels), (48000, 8));
    assert_eq!(program.unwrap().channels(), 8);
    assert!(AacConfig::parse(asc).is_err());
}
