use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
#[test]
fn original_video_with_he_aac_audio_demuxes_and_decodes_the_intended_sbr_failure() {
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4");
    let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/he-aac-sbr-packets.json"
    ))
    .unwrap();
    let mut reader =
        Mp4Reader::open(std::io::Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
    assert_eq!(reader.tracks().len(), 2);
    assert!(reader.refused().is_empty());
    assert!(
        reader
            .tracks()
            .iter()
            .any(|t| t.handler == *b"vide" && t.codec == *b"avc1" && !t.samples.is_empty())
    );
    let index = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let audio = &reader.tracks()[index];
    assert_eq!(
        (audio.sample_rate, audio.channels, audio.samples.len()),
        (48000, 1, 3)
    );
    let asc = config::aac_specific_config(&audio.configuration).unwrap();
    let parsed = config::AudioSpecificConfig::parse(asc).unwrap();
    assert_eq!(
        (
            parsed.core.sample_rate,
            parsed.output_sample_rate(),
            parsed.sbr_present
        ),
        (24000, 48000, Some(true))
    );
    // The compatibility LC metadata API intentionally remains a refusal; the
    // extension-aware native decoder is the acceptance engine for this fix.
    assert!(config::AacConfig::parse(asc).is_err());
    let mut adapter =
        fvid::codec::aac_decoder::AacDecoder::new(&audio.configuration, 48000, 1).unwrap();
    assert_eq!(adapter.spec().sample_rate, 48000);
    let mut decoder = NativeAacDecoder::new(asc).unwrap();
    let mut output = Vec::new();
    let mut packet = Vec::new();
    for i in 0..3 {
        reader.read_packet(index, i, &mut packet).unwrap();
        let decoded = decoder.decode(&packet).unwrap();
        let adapted = adapter
            .decode(&packet, i as u64 * 2048, 2048)
            .unwrap()
            .unwrap();
        assert_eq!(
            (adapted.pts, adapted.timebase_num, adapted.timebase_den),
            (i as u64 * 2048, 1, 48000)
        );
        assert_eq!(
            adapted.data,
            decoded
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<_>>()
        );
        output.extend(decoded);
    }
    assert_eq!(output.len(), 6144);
    assert!(output.iter().any(|v| v.abs() > 1e-5));
    let raw = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
    let offset = manifest["video"]["pcm_offset"].as_u64().unwrap() as usize;
    for (&value, b) in output
        .iter()
        .zip(raw[offset..offset + output.len() * 8].chunks_exact(8))
    {
        assert_eq!(
            value.to_bits(),
            (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_bits()
        );
    }
}

#[test]
fn he_aac_player_and_export_clocks_preserve_full_pcm_and_seek() {
    use std::{io::Cursor, time::Duration};
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4");
    let mut full = Vec::new();
    let reader = Mp4Reader::open(Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
    let stats = fvid::native_media::decode_mp4_aac_reader(reader, &mut full, None).unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (48000, 1, 6144)
    );
    let mut exported = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(fixture.as_slice()),
        &mut exported,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 6144);
    assert_eq!(full, exported);
    for (from, to) in [(0, 32), (32, 96), (64, 128), (0, 128)] {
        let interval = Some((Duration::from_millis(from), Duration::from_millis(to)));
        let mut seek = Vec::new();
        let reader = Mp4Reader::open(Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
        fvid::native_media::decode_mp4_aac_reader(reader, &mut seek, interval).unwrap();
        assert_eq!(seek, &full[from as usize * 48 * 4..to as usize * 48 * 4]);
        let mut export = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(fixture.as_slice()),
            &mut export,
            interval,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(seek, export);
    }
}

#[test]
fn he_aac_remux_uses_output_sample_clock_and_preserves_packets() {
    use std::io::Cursor;
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4");
    let mut source =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(fixture.as_slice()), Default::default())
            .unwrap();
    let mut output = Cursor::new(Vec::new());
    fvid_media::owned_mp4_matroska::write(&mut source, &mut output, None, None).unwrap();
    output.set_position(0);
    let mut reader = fvid_media::owned_webm::WebmReader::open(output, Default::default()).unwrap();
    reader.scan_all().unwrap();
    let audio = reader
        .tracks
        .iter()
        .find(|t| t.sample_rate == 48000)
        .unwrap();
    let packets: Vec<_> = reader
        .packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.track == audio.number)
        .map(|(i, p)| (i, p.pts_ns))
        .collect();
    assert_eq!(packets.len(), 3);
    for (n, (i, pts)) in packets.into_iter().enumerate() {
        assert!((pts as i64 - (n as i64 * 2048 * 1_000_000_000 / 48000)).abs() < 1000);
        let audio_index = source
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut original = Vec::new();
        source.read_packet(audio_index, n, &mut original).unwrap();
        assert_eq!(reader.read_packet(i).unwrap(), original);
    }
}

#[test]
fn he_aac_matroska_seek_and_controlled_export_preserve_pcm() {
    use std::{io::Cursor, time::Duration};
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4");
    let generous = fvid_media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let tight = fvid_media::CopyOptions {
        max_controlled_bytes: Some(1024),
        ..Default::default()
    };
    let mut full = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(fixture.as_slice()),
        &mut full,
        None,
        &generous,
    )
    .unwrap();
    let mut refused = Vec::new();
    let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(fixture.as_slice()),
        &mut refused,
        None,
        &tight,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("controlled memory budget exceeded")
    );
    assert!(refused.is_empty());
    let mut source =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(fixture.as_slice()), Default::default())
            .unwrap();
    let mut output = Cursor::new(Vec::new());
    fvid_media::owned_mp4_matroska::write(&mut source, &mut output, None, None).unwrap();
    let bytes = output.into_inner();
    for interval in [
        None,
        Some((Duration::from_millis(32), Duration::from_millis(96))),
        Some((Duration::ZERO, Duration::from_millis(128))),
    ] {
        let mut pcm = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            interval,
            &generous,
        )
        .unwrap();
        let expected = if interval.is_some_and(|(from, _)| !from.is_zero()) {
            &full[32 * 48 * 4..96 * 48 * 4]
        } else {
            full.as_slice()
        };
        assert_eq!(pcm, expected);
        let reader =
            fvid::container::webm::WebmReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        let mut player = Vec::new();
        fvid::native_media::decode_matroska_aac_reader(reader, &mut player, interval).unwrap();
        assert_eq!(player, expected);
    }
    let mut refused = Vec::new();
    let error = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
        Cursor::new(&bytes),
        &mut refused,
        None,
        &tight,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("controlled memory budget exceeded")
    );
    assert!(refused.is_empty());
}

#[test]
fn original_stereo_he_aac_video_preserves_both_channels_in_player_and_export() {
    use std::io::Cursor;
    let stereo = include_bytes!("fixtures/playback-errors/he-aac-sbr-stereo.mp4");
    let mono = include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4");
    let mut reference = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(mono.as_slice()),
        &mut reference,
        None,
        &Default::default(),
    )
    .unwrap();
    let mut output = Vec::new();
    let reader = Mp4Reader::open(Cursor::new(stereo.as_slice()), Limits::default()).unwrap();
    let stats = fvid::native_media::decode_mp4_aac_reader(reader, &mut output, None).unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (48000, 2, 6144)
    );
    for (pair, sample) in output.chunks_exact(8).zip(reference.chunks_exact(4)) {
        assert_eq!(&pair[..4], sample);
        assert_eq!(&pair[4..], sample);
    }
    assert_eq!(output.len(), reference.len() * 2);
    let mut exported = Vec::new();
    let options = fvid_media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(stereo.as_slice()),
        &mut exported,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(output, exported);
}

#[test]
fn synthetic_video_with_missing_sbr_fill_decodes_and_seeks_without_clock_changes() {
    use std::{io::Cursor, time::Duration};
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-missing-sbr.mp4");
    let metadata: serde_json::Value = serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/he-aac-missing-sbr.json"
    ))
    .unwrap();
    let mut pcm = Vec::new();
    let options = fvid_media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(fixture.as_slice()),
        &mut pcm,
        None,
        &options,
    )
    .unwrap();
    assert_eq!((stats.sample_rate, stats.sample_frames), (48000, 6144));
    let offset = metadata["video"]["pcm_offset"].as_u64().unwrap() as usize;
    let expected = include_bytes!("fixtures/playback-errors/he-aac-missing-sbr.f64le");
    for (actual, b) in pcm.chunks_exact(4).zip(expected[offset..].chunks_exact(8)) {
        assert!(
            (f32::from_le_bytes(actual.try_into().unwrap())
                - (f64::from_le_bytes(b.try_into().unwrap()) as f32))
                .abs()
                < 1e-7
        );
    }
    for (from, to) in [(0, 128), (32, 96), (64, 128), (0, 32)] {
        let mut player = Vec::new();
        let reader = Mp4Reader::open(Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
        fvid::native_media::decode_mp4_aac_reader(
            reader,
            &mut player,
            Some((Duration::from_millis(from), Duration::from_millis(to))),
        )
        .unwrap();
        assert_eq!(player, &pcm[from as usize * 48 * 4..to as usize * 48 * 4]);
    }
}

#[test]
fn implicit_sbr_video_uses_declared_clock_in_player_export_and_remux() {
    use std::{io::Cursor, time::Duration};
    let bytes = include_bytes!("fixtures/playback-errors/he-aac-implicit-sbr.mp4");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/he-aac-implicit-sbr.mp4");
    let info = fvid::native_media::aac_source_info(&path).unwrap();
    assert_eq!((info.sample_rate, info.channels), (48000, 1));
    let reader = Mp4Reader::open(Cursor::new(bytes.as_slice()), Limits::default()).unwrap();
    let track = reader
        .tracks()
        .iter()
        .find(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(track.sample_rate, 48000);
    let asc = config::aac_specific_config(&track.configuration).unwrap();
    assert_eq!(
        config::AudioSpecificConfig::parse(asc).unwrap().sbr_present,
        None
    );
    let mut player = Vec::new();
    fvid::native_media::decode_mp4_aac_reader(reader, &mut player, None).unwrap();
    let mut exported = Vec::new();
    let options = fvid_media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes.as_slice()),
        &mut exported,
        None,
        &options,
    )
    .unwrap();
    assert_eq!((stats.sample_rate, stats.sample_frames), (48000, 6144));
    assert_eq!(player, exported);
    let mut explicit = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(include_bytes!("fixtures/playback-errors/he-aac-sbr-synthetic.mp4").as_slice()),
        &mut explicit,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(player, explicit);
    let mut source =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes.as_slice()), Default::default())
            .unwrap();
    let mut mkv = Cursor::new(Vec::new());
    fvid_media::owned_mp4_matroska::write(&mut source, &mut mkv, None, None).unwrap();
    let mut decoded = Vec::new();
    fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
        Cursor::new(mkv.get_ref()),
        &mut decoded,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(player, decoded);
    let reader =
        fvid::container::webm::WebmReader::open(Cursor::new(mkv.get_ref()), Default::default())
            .unwrap();
    let mut seek = Vec::new();
    fvid::native_media::decode_matroska_aac_reader(
        reader,
        &mut seek,
        Some((Duration::from_millis(32), Duration::from_millis(96))),
    )
    .unwrap();
    assert_eq!(seek, &player[32 * 48 * 4..96 * 48 * 4]);
}

#[test]
fn original_adts_packets_discover_sbr_and_match_the_paired_video_pcm() {
    use std::io::Cursor;
    let bytes = include_bytes!("fixtures/playback-errors/he-aac-implicit-sbr.aac");
    let mut reader = fvid::container::adts::StreamReader::open(bytes.as_slice()).unwrap();
    assert_eq!(reader.configuration().sample_rate, 24000);
    let mut decoder =
        NativeAacDecoder::new_with_sbr_detection(reader.audio_specific_config()).unwrap();
    let mut pcm = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        pcm.extend(decoder.decode(&packet).unwrap());
        assert_eq!(decoder.sample_rate(), 48000);
    }
    assert_eq!(pcm.len(), 6144);
    let mut video = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(include_bytes!("fixtures/playback-errors/he-aac-implicit-sbr.mp4").as_slice()),
        &mut video,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        pcm.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),
        video
    );
    let initial = NativeAacDecoder::new_with_sbr_detection(&[0x13, 0x08])
        .unwrap()
        .checkpoint();
    decoder.restore(&initial).unwrap();
    assert_eq!(decoder.sample_rate(), 24000);
}
