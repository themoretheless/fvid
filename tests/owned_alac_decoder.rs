#[test]
fn library_alac_reconstruction_matches_frontend_for_every_fixture_packet() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/alac");
    for name in ["mono-16.m4a", "mono-24.m4a", "stereo-16.m4a", "stereo-24.m4a", "stereo-pair-24.m4a", "silence-16.m4a", "noise-24.m4a"] {
        let mut reader = fvid::container::mp4::Mp4Reader::open(
            std::fs::File::open(fixtures.join(name)).unwrap(), Default::default()).unwrap();
        let track = reader.tracks()[0].clone();
        assert_eq!(track.codec, *b"alac");
        let mut library = fvid_media::owned_alac::AlacDecoder::new(
            &track.configuration, track.sample_rate, track.channels).unwrap();
        assert_eq!((library.sample_rate(), library.channels()), (track.sample_rate, track.channels));
        let mut frontend = fvid::codec::alac_decoder::AlacDecoder::new(
            &track.configuration, track.sample_rate, track.channels).unwrap();
        let mut data = Vec::new();
        assert!(track.samples.len() > 0);
        for index in 0..track.samples.len() {
            reader.read_packet(0, index, &mut data).unwrap();
            let expected = frontend.decode_pcm(&data).unwrap();
            let actual = library.decode_pcm(&data).unwrap();
            assert_eq!(actual.len() % usize::from(track.channels), 0);
            assert!(!actual.is_empty());
            assert_eq!(actual, expected, "{name} packet {index}");
            // ALAC packets are self-contained: the same frame is independent
            // of prior decode order, as required for seek.
            assert_eq!(library.decode_pcm(&data).unwrap(), actual);
        }
    }
    assert!(fvid_media::owned_alac::AlacDecoder::new(&[], 48000, 2).is_err());
}

#[test]
fn library_matroska_alac_packets_match_frontend_without_container_adapter() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/alac/stereo-24.mka");
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(source).unwrap(), Default::default()).unwrap();
    reader.scan_all().unwrap();
    let track = reader.tracks.iter().find(|track| track.codec == "A_ALAC").unwrap().clone();
    let mut library = fvid_media::owned_alac::AlacDecoder::from_matroska(&track).unwrap();
    let mut frontend = fvid::codec::alac_decoder::AlacDecoder::new(
        &track.codec_private, library.sample_rate(), library.channels()).unwrap();
    let mut count = 0;
    for index in 0..reader.packets.len() {
        if reader.packets[index].track != track.number { continue; }
        let packet = reader.read_packet(index).unwrap();
        assert_eq!(library.decode_pcm(&packet).unwrap(), frontend.decode_pcm(&packet).unwrap());
        count += 1;
    }
    assert!(count > 0);
    for (kind, codec, rate, channels) in [
        (1, "A_ALAC", 48000, 2), (2, "A_AAC", 48000, 2),
        (2, "A_ALAC", u64::MAX, 2), (2, "A_ALAC", 48000, u64::MAX),
    ] {
        let mut invalid = track.clone();
        invalid.kind = kind; invalid.codec = codec.into();
        invalid.sample_rate = rate; invalid.channels = channels;
        assert!(fvid_media::owned_alac::AlacDecoder::from_matroska(&invalid).is_err());
    }
}
