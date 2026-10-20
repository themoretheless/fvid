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
        assert!(!track.samples.is_empty());
        for index in 0..track.samples.len() {
            reader.read_packet(0, index, &mut data).unwrap();
            let expected = frontend.decode_pcm(&data).unwrap();
            let actual = library.decode_pcm(&data).unwrap();
            assert_eq!(actual.len() % usize::from(track.channels), 0);
            assert!(!actual.is_empty());
            assert_eq!(actual, expected, "{name} packet {index}");
            let frames=actual.len()/usize::from(track.channels);
            assert_eq!(fvid_media::owned_alac::AlacDecoder::packet_samples(&track.configuration,track.sample_rate,track.channels,&data).unwrap(),frames,"{name} header {index}");
            assert_eq!(fvid::codec::alac_decoder::AlacDecoder::packet_samples(&track.configuration,track.sample_rate,track.channels,&data).unwrap(),frames);
            for length in 0..3 {assert!(fvid_media::owned_alac::AlacDecoder::packet_samples(&track.configuration,track.sample_rate,track.channels,&data[..length]).is_err());}

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

#[test]
fn synthetic_matroska_alac_cookie_rate_must_match_track_geometry() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors");
    let control = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(fixtures.join("alac-control.mka")).unwrap(), Default::default()).unwrap();
    let mut decoder = fvid_media::owned_alac::AlacDecoder::from_matroska(&control.tracks[0]).unwrap();
    let mut control = control;
    control.scan_all().unwrap();
    assert_eq!(decoder.decode_pcm(&control.read_packet(0).unwrap()).unwrap(), vec![1.0 / 32768.0]);
    let mismatch = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(fixtures.join("alac-rate-mismatch.mka")).unwrap(), Default::default()).unwrap();
    let error = match fvid_media::owned_alac::AlacDecoder::from_matroska(&mismatch.tracks[0]) {
        Ok(_) => panic!("accepted ALAC cookie/track sample-rate mismatch"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("sample rates disagree"));
}

#[test]
fn every_alac_entry_point_agrees_on_the_cookie_sample_clock() {
    use fvid::codec::alac_decoder::AlacDecoder as Frontend;
    use fvid_media::owned_alac::AlacDecoder as Library;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel");
    let cookie = std::fs::read(root.join("stereo32.cookie")).unwrap();
    let packet = std::fs::read(root.join("stereo32.frame")).unwrap();
    assert!(Frontend::new(&cookie,48000,2).is_ok());
    assert!(Library::new(&cookie,48000,2).is_ok());
    assert_eq!(Frontend::packet_samples(&cookie,48000,2,&packet).unwrap(),16);
    assert_eq!(Library::packet_samples(&cookie,48000,2,&packet).unwrap(),16);
    for rate in [44100u32,96000] {
        let mut matching = cookie.clone();
        matching[20..24].copy_from_slice(&rate.to_be_bytes());
        assert!(Frontend::new(&matching,rate,2).is_ok());
        assert!(Library::new(&matching,rate,2).is_ok());
        assert_eq!(Frontend::packet_samples(&matching,rate,2,&packet).unwrap(),16);
        assert_eq!(Library::packet_samples(&matching,rate,2,&packet).unwrap(),16);
        assert!(Frontend::new(&cookie,rate,2).is_err());
        assert!(Library::new(&cookie,rate,2).is_err());
        assert!(Frontend::configuration_mask(&cookie,rate,2).is_err());
        assert!(Library::configuration_mask(&cookie,rate,2).is_err());
        assert!(Frontend::packet_samples(&cookie,rate,2,&packet).is_err());
        assert!(Library::packet_samples(&cookie,rate,2,&packet).is_err());
        assert!(Frontend::decode_admission_bytes(&cookie,rate,2).is_err());
        assert!(Library::decode_admission_bytes(&cookie,rate,2).is_err());
    }
    let mut zero=cookie;zero[20..24].fill(0);
    assert!(Frontend::new(&zero,48000,2).is_err());
    assert!(Library::new(&zero,48000,2).is_err());
}

#[test]
fn unknown_alac_compatible_versions_refuse_across_all_shared_entry_points() {
    use fvid::codec::alac_decoder::AlacDecoder as Frontend;
    use fvid_media::owned_alac::AlacDecoder as Library;
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/alac-multichannel");
    let cookie=std::fs::read(root.join("stereo32.cookie")).unwrap();
    let packet=std::fs::read(root.join("stereo32.frame")).unwrap();
    assert!(Frontend::new(&cookie,48000,2).is_ok());assert!(Library::new(&cookie,48000,2).is_ok());
    for version in [1u8,255] {
        let mut raw=cookie.clone();raw[4]=version;
        let mut wrapped=vec![0,0,0,12,b'f',b'r',b'm',b'a',b'a',b'l',b'a',b'c',0,0,0,36,b'a',b'l',b'a',b'c',0,0,0,0];wrapped.extend(&raw);wrapped.extend([0,0,0,8,0,0,0,0]);
        for setup in [&raw,&wrapped] {
            let errors=[
                Frontend::new(setup,48000,2).err().unwrap().to_string(),
                Library::new(setup,48000,2).err().unwrap().to_string(),
                Frontend::packet_samples(setup,48000,2,&packet).unwrap_err().to_string(),
                Library::packet_samples(setup,48000,2,&packet).unwrap_err().to_string(),
                Frontend::configuration_mask(setup,48000,2).unwrap_err().to_string(),
                Library::configuration_mask(setup,48000,2).unwrap_err().to_string(),
                Frontend::decode_admission_bytes(setup,48000,2).unwrap_err().to_string(),
                Library::decode_admission_bytes(setup,48000,2).unwrap_err().to_string(),
            ];
            for error in errors {assert!(error.contains(&format!("unsupported ALAC compatible version {version}")),"{error}");}
        }
    }
}

#[test]
fn full_width_alac_integer_output_preserves_low_bits_and_extrema() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-integer-output");
    let bytes = std::fs::read(root.join("expected.i32le")).unwrap();
    let expected: Vec<i32> = bytes.chunks_exact(4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect();
    assert_ne!(expected[0], expected[1]);
    assert_eq!(expected[0] as f32, expected[1] as f32);
    assert!(expected.contains(&i32::MIN) && expected.contains(&i32::MAX));
    for name in ["control.m4a", "rice.m4a", "predictor.m4a"] {
    let expected = if name.starts_with("predictor") {
        std::fs::read(root.join("predictor.i32le")).unwrap().chunks_exact(4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>()
    } else { expected.clone() };
    let mut reader = fvid::container::mp4::Mp4Reader::open(
        std::fs::File::open(root.join(name)).unwrap(), Default::default()).unwrap();
    let track = reader.tracks()[0].clone();
    let mut library = fvid_media::owned_alac::AlacDecoder::new(
        &track.configuration, track.sample_rate, track.channels).unwrap();
    let mut frontend = fvid::codec::alac_decoder::AlacDecoder::new(
        &track.configuration, track.sample_rate, track.channels).unwrap();
    let mut packet = Vec::new();
    for index in 0..track.samples.len() {
        reader.read_packet(0,index,&mut packet).unwrap();
        let target = &expected[index*16..(index+1)*16];
        assert_eq!(library.decode_pcm_i32(&packet).unwrap(), target);
        assert_eq!(frontend.decode_pcm_i32(&packet).unwrap(), target);
        assert_eq!(library.decode_pcm_i32(&packet).unwrap(), target);
    }
    }
    for name in ["control.mkv", "rice.mkv", "predictor.mkv"] {
        let expected = if name.starts_with("predictor") {
            std::fs::read(root.join("predictor.i32le")).unwrap().chunks_exact(4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>()
        } else { expected.clone() };
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            std::fs::File::open(root.join(name)).unwrap(), Default::default()).unwrap();
        reader.scan_all().unwrap();
        let track = reader.tracks.iter().find(|t| t.codec == "A_ALAC").unwrap().clone();
        let mut decoder = fvid_media::owned_alac::AlacDecoder::from_matroska(&track).unwrap();
        let mut actual = Vec::new();
        for index in 0..reader.packets.len() {
            if reader.packets[index].track == track.number {
                actual.extend(decoder.decode_pcm_i32(&reader.read_packet(index).unwrap()).unwrap());
            }
        }
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn integer_alac_output_preserves_depth_and_channel_order_across_authored_corpus() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap().replace(".mkv", ".m4a");
        let depth = case["depth"].as_u64().unwrap() as u32;
        let total = case["samples"].as_u64().unwrap_or(32) as usize;
        let mut expected = Vec::new();
        for n in 0..total {
            for channel in case["order"].as_array().unwrap() {
                let channel = channel.as_u64().unwrap() as i32;
                let base = ((channel+1)*101+n as i32*7)
                    * if channel%2==0 { 1 } else { -1 };
                let jump = if case["jump_sample"].as_u64().is_some_and(|t| n as u64>=t) {
                    case["jump_high"].as_i64().unwrap() as i32
                } else { 0 };
                let value = (base+jump)*case["scale"].as_i64().unwrap_or(1) as i32
                    + if case["low_bias"].as_bool().unwrap_or(false) {channel+1} else {0};
                expected.push(value.wrapping_shl(32-depth));
            }
        }
        let mut reader = fvid::container::mp4::Mp4Reader::open(
            std::fs::File::open(root.join(&name)).unwrap(), Default::default()).unwrap();
        let track = reader.tracks()[0].clone();
        let mut library = fvid_media::owned_alac::AlacDecoder::new(
            &track.configuration, track.sample_rate, track.channels).unwrap();
        let mut frontend = fvid::codec::alac_decoder::AlacDecoder::new(
            &track.configuration, track.sample_rate, track.channels).unwrap();
        let mut actual = Vec::new();
        let mut packet = Vec::new();
        for index in 0..track.samples.len() {
            reader.read_packet(0,index,&mut packet).unwrap();
            let decoded = library.decode_pcm_i32(&packet).unwrap();
            assert_eq!(frontend.decode_pcm_i32(&packet).unwrap(), decoded, "{name}/{index}");
            actual.extend(decoded);
        }
        assert_eq!(actual,expected,"{name}");
    }
}

#[test]
fn full_width_adaptive_alac_preserves_literal_pcm() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-full-width-adaptive");
    for order in [1,2,4,8,16,30] {
        for sign in [-1,1] {
            let name = format!("order{order}-{}",if sign<0 {"negative"} else {"positive"});
            let expected: Vec<i32> = (0..256).map(|n|
                sign*(0x40000001+n*7+if n>=80 {23} else {0})).collect();
            let mut video = fvid::playback_native::NativeReader::software(
                std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mkv"))).unwrap()),
                32 << 20).unwrap();
            assert!(video.read_frame_raw().unwrap().is_some());
            assert!(video.read_frame_raw().unwrap().is_some());
            assert!(video.read_frame_raw().unwrap().is_none());
            let cookie = std::fs::read(root.join(format!("{name}.cookie"))).unwrap();
            let mut decoder = fvid_media::owned_alac::AlacDecoder::new(&cookie,48000,1).unwrap();
            for index in 0..2 {
                let packet = std::fs::read(root.join(format!("{name}.packet{index}"))).unwrap();
                assert_eq!(decoder.decode_pcm_i32(&packet).unwrap(),
                    expected[index*128..(index+1)*128],"{name}/{index}");
            }
        }
    }
}

#[test]
fn full_width_stereo_alac_preserves_matrix_and_varying_low_bytes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-stereo-integer");
    let expected: Vec<i32> = (0..32).flat_map(|n| {
        let sign = if n%2==0 {1} else {-1};
        [((sign*(0x400001+n*7))<<8)|(n*13%256),
         ((-sign*(0x400013+n*3))<<8)|((255i32-n*17).rem_euclid(256))]
    }).collect();
    assert_eq!(std::fs::read(root.join("expected.i32le")).unwrap(),
        expected.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>());
    for extra in [8,16] {
    for weight in [-128,-127,-64,-1,0,1,64,127] {
        for shift in [7,31] {
            let name = format!("{}weight{weight}-shift{shift}",if extra==8 {""} else {"low16-"});
            let mut reader = fvid::container::mp4::Mp4Reader::open(
                std::fs::File::open(root.join(format!("{name}.m4a"))).unwrap(),
                Default::default()).unwrap();
            let track = reader.tracks()[0].clone();
            let mut decoder = fvid_media::owned_alac::AlacDecoder::new(
                &track.configuration,48000,2).unwrap();
            let mut packet = Vec::new();
            for index in 0..2 {
                reader.read_packet(0,index,&mut packet).unwrap();
                assert_eq!(decoder.decode_pcm_i32(&packet).unwrap(),
                    expected[index*32..(index+1)*32],"{name}/{index}");
            }
            let mut video = fvid::playback_native::NativeReader::software(
                std::io::Cursor::new(std::fs::read(root.join(format!("{name}.mkv"))).unwrap()),
                32 << 20).unwrap();
            assert!(video.read_frame_raw().unwrap().is_some());
            assert!(video.read_frame_raw().unwrap().is_some());
            assert!(video.read_frame_raw().unwrap().is_none());
        }
    }
    }
}
