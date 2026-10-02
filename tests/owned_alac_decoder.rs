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
