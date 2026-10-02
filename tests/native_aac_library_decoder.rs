#[test]
fn library_and_frontend_decode_identical_packets_with_identical_pcm() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
        "aac-pce-wide8.aac",
        "aac-tns.aac",
    ] {
        let bytes = std::fs::read(fixtures.join(name)).unwrap();
        let source = fvid_media::owned_aac::adts::Aac::parse(&bytes, &Default::default()).unwrap();
        let mut library =
            fvid_media::owned_aac::NativeAacDecoder::new(&source.configuration).unwrap();
        let mut frontend =
            fvid::codec::aac_native::NativeAacDecoder::new(&source.configuration).unwrap();
        assert_eq!(
            (
                library.channels(),
                library.sample_rate(),
                library.channel_mask()
            ),
            (
                frontend.channels(),
                frontend.sample_rate(),
                frontend.channel_mask()
            )
        );
        for packet in 0..source.packets() {
            let expected = frontend.decode(source.packet(packet)).unwrap();
            assert_eq!(
                library.decode(source.packet(packet)).unwrap(),
                expected,
                "{name} packet {packet}"
            );
        }
    }
}
