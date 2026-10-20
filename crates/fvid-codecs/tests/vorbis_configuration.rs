use fvid_codecs::vorbis_audio::configuration_headers;
#[test]
fn xiph_laced_configuration_uses_lengths_instead_of_payload_marker_searches() {
    let bytes = include_bytes!("../../../tests/fixtures/ogg/vorbis-stereo.ogg");
    let ogg = fvid_media::owned_ogg::Ogg::parse(bytes, &Default::default()).unwrap();
    let packets = &ogg.chains()[0].packets;
    let ident = &packets[0].data;
    let setup = &packets[2].data;
    let mut comment = b"\x03vorbis".to_vec();
    comment.extend(vec![0; 600]);
    comment.extend_from_slice(b"\x05vorbis");
    let mut configuration = vec![2];
    for size in [ident.len(), comment.len()] {
        configuration.extend(vec![255; size / 255]);
        configuration.push((size % 255) as u8);
    }
    configuration.extend(ident);
    configuration.extend(&comment);
    configuration.extend(setup);
    let (actual_ident, actual_setup) = configuration_headers(&configuration).unwrap();
    assert_eq!(actual_ident, ident);
    assert_eq!(actual_setup, setup);
    let concatenated = [ident.as_slice(), setup.as_slice()].concat();
    let (a, b) = configuration_headers(&concatenated).unwrap();
    assert_eq!(a, ident);
    assert_eq!(b, setup);
    assert!(configuration_headers(&[2, 255]).is_err());
    assert!(configuration_headers(&[2, 30, 255, 255, 1]).is_err());
}
