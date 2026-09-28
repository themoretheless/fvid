use std::io::Cursor;

#[test]
fn playback_domain_namespace_and_legacy_path_name_the_same_reader() {
    type DomainReader = fvid::playback::video::native::NativeReader<Cursor<&'static [u8]>>;
    type LegacyReader = fvid::playback_native::NativeReader<Cursor<&'static [u8]>>;
    assert_eq!(
        std::any::type_name::<DomainReader>(),
        std::any::type_name::<LegacyReader>()
    );
}

#[cfg(feature = "player")]
#[test]
fn playback_audio_namespace_reexports_the_legacy_module() {
    type DomainReader = fvid::playback::audio::wav::WavAudioReader;
    type LegacyReader = fvid::playback_wav::WavAudioReader;
    assert_eq!(
        std::any::type_name::<DomainReader>(),
        std::any::type_name::<LegacyReader>()
    );
}

#[test]
fn y4m_namespace_and_root_facade_share_the_same_api() {
    let header = b"YUV4MPEG2 W4 H4 C420\n";
    let legacy = fvid::Header::parse(header).unwrap();
    let domain = fvid::y4m::Header::parse(header).unwrap();
    assert_eq!(legacy.width, domain.width);
    assert_eq!(legacy.height, domain.height);
    assert_eq!(legacy.format, domain.format);

    let mut source = header.to_vec();
    source.extend_from_slice(b"FRAME\n");
    source.extend(0..24);

    let mut legacy_output = Vec::new();
    fvid::process(
        Cursor::new(&source),
        &mut legacy_output,
        fvid::Transform::default(),
        1024,
    )
    .unwrap();

    let mut domain_output = Vec::new();
    fvid::y4m::process(
        Cursor::new(&source),
        &mut domain_output,
        fvid::y4m::Transform::default(),
        1024,
    )
    .unwrap();

    assert_eq!(domain_output, legacy_output);
}
