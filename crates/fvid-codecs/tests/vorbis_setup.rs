use fvid_codecs::vorbis::{Floor, Identification, PacketBits, PacketMode, Setup, SetupLimits};

#[test]
fn saved_synthetic_vorbis_tone_setup_and_every_packet_mode_are_owned() {
    let bytes = include_bytes!("../../../tests/fixtures/ogg/vorbis-stereo.ogg");
    let ogg = fvid_media::owned_ogg::Ogg::parse(bytes, &Default::default()).unwrap();
    let chain = &ogg.chains()[0];
    let id = Identification::parse(&chain.packets[0].data).unwrap();
    let setup = Setup::parse(
        &chain.packets[2].data,
        id.channels,
        SetupLimits {
            entries: 1_000_000,
            nodes: 2_000_000,
            multiplicands: 1_000_000,
        },
    )
    .unwrap();
    assert!(!setup.codebooks.is_empty());
    assert!(!setup.floors.is_empty());
    assert!(!setup.residues.is_empty());
    assert!(!setup.mappings.is_empty());
    assert!(!setup.modes.is_empty());
    assert!(chain.packets.len() > 10);
    let mut used_floors = 0;
    for packet in &chain.packets[3..] {
        let mut bits = PacketBits::new(&packet.data);
        let header = PacketMode::parse(&mut bits, &setup.modes).unwrap();
        assert!(header.mode < setup.modes.len());
        let mapping = &setup.mappings[usize::from(setup.modes[header.mode].mapping)];
        let floor = &setup.floors[usize::from(mapping.submaps[usize::from(mapping.mux[0])].floor)];
        let Floor::One(floor) = floor else {
            panic!("synthetic tone should exercise floor one")
        };
        if let Some(y) = floor.decode_y(&mut bits, &setup.codebooks).unwrap() {
            let mut curve =
                vec![
                    0;
                    1usize << (id.block_exponents[usize::from(setup.modes[header.mode].long)] - 1)
                ];
            floor.curve_indices(&y, &mut curve).unwrap();
            let mut linear = vec![0.; curve.len()];
            floor.curve(&y, &mut linear, &mut curve).unwrap();
            assert!(linear.iter().all(|&v| v.is_finite() && v > 0. && v <= 1.));
            used_floors += 1;
        }
    }
    assert!(used_floors > 10);
}

#[test]
fn owned_vorbis_pcm_preserves_synthetic_tone_frequency_and_reset() {
    let bytes = include_bytes!("../../../tests/fixtures/ogg/vorbis-stereo.ogg");
    let ogg = fvid_media::owned_ogg::Ogg::parse(bytes, &Default::default()).unwrap();
    let chain = &ogg.chains()[0];
    let id = Identification::parse(&chain.packets[0].data).unwrap();
    let setup = Setup::parse(
        &chain.packets[2].data,
        id.channels,
        SetupLimits {
            entries: 1_000_000,
            nodes: 2_000_000,
            multiplicands: 1_000_000,
        },
    )
    .unwrap();
    let mut decoder = fvid_codecs::vorbis_audio::Decoder::new(id, setup, 64 * 1024 * 1024).unwrap();
    assert_eq!(decoder.channels(), id.channels);
    assert_eq!(decoder.sample_rate(), id.sample_rate);
    let mut pcm = Vec::new();
    for (index, packet) in chain.packets[3..].iter().enumerate() {
        let samples = decoder.decode(&packet.data).unwrap();
        if index == 0 {
            assert!(samples.is_empty(), "first packet primes overlap");
        }
        pcm.extend(samples);
    }
    assert!(pcm.iter().all(|v| v.is_finite()));
    assert_eq!(pcm.len(), 178176);
    let rms = (pcm.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
    assert!((0.05..0.08).contains(&rms), "rms={rms}");
    let channels = usize::from(id.channels);
    let mono: Vec<_> = pcm
        .chunks_exact(channels)
        .skip(4096)
        .take(id.sample_rate as usize)
        .map(|s| s[0])
        .collect();
    let crossings = mono.windows(2).filter(|s| s[0] <= 0. && s[1] > 0.).count();
    assert!(
        (438..=442).contains(&crossings),
        "440 Hz synthetic tone: crossings={crossings}"
    );
    decoder.reset();
    let mut repeated = Vec::new();
    for packet in &chain.packets[3..] {
        repeated.extend(decoder.decode(&packet.data).unwrap());
    }
    assert_eq!(pcm, repeated);
}
