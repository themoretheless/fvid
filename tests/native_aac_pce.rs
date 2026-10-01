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

#[test]
fn tagged_eight_channel_decode_matches_independent_pcm_per_speaker() {
    use fvid::{
        codec::{aac_native::NativeAacDecoder, config::aac_specific_config},
        container::mp4::Mp4Reader,
    };
    let mut reader = Mp4Reader::open(
        std::io::Cursor::new(include_bytes!("fixtures/audio/aac-pce-wide8.m4a")),
        Default::default(),
    )
    .unwrap();
    let track = reader.tracks()[0].clone();
    let mut decoder =
        NativeAacDecoder::new(aac_specific_config(&track.configuration).unwrap()).unwrap();
    assert_eq!(decoder.channel_mask(), 0xff);
    let mut first_packet = Vec::new();
    reader.read_packet(0, 0, &mut first_packet).unwrap();
    let first = decoder.decode(&first_packet).unwrap();
    decoder.reset();
    assert!(decoder.decode(&[0xa0]).is_err()); // Truncated in-band PCE must not alter synthesis.
    assert_eq!(decoder.decode(&first_packet).unwrap(), first);
    decoder.reset();
    let mut pcm = Vec::new();
    let mut packet = Vec::new();
    for index in 0..track.samples.len() {
        reader.read_packet(0, index, &mut packet).unwrap();
        pcm.extend(decoder.decode(&packet).unwrap());
    }
    let reference = include_bytes!("fixtures/audio/aac-pce-wide8-mp4-reference.f32le");
    // The reference ignores container edits, matching raw packet synthesis.
    let reference: Vec<f32> = reference
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();

    let last = track.samples.get(track.samples.len() - 1).unwrap();
    assert_eq!(
        pcm.len() - reference.len(),
        (1024 - last.duration as usize) * 8
    );
    let mut squared = [0.0f64; 8];
    let mut peak = [0.0f64; 8];
    for (i, (&a, &b)) in pcm.iter().zip(&reference).enumerate() {
        let e = f64::from(a) - f64::from(b);
        squared[i % 8] += e * e;
        peak[i % 8] = peak[i % 8].max(e.abs());
    }
    for ch in 0..8 {
        let rms = (squared[ch] / (reference.len() / 8) as f64).sqrt();
        assert!(
            rms < 1e-7 && peak[ch] < 1e-6,
            "channel {ch}: {rms}, {}",
            peak[ch]
        );
    }
}
