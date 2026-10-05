use fvid::{codec::aac_native::NativeAacDecoder, container::adts::StreamReader};
use std::io::Cursor;
#[test]
fn coupling_matches_saved_pcm_reset_and_checkpoint() {
    for (data, expected, channels) in [
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling.f32le").as_slice(),
            1,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo.aac")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo.f32le")
                .as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-shared.aac")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-shared.f32le")
                .as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-right.aac")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-right.f32le")
                .as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-left.aac")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-left.f32le")
                .as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short.aac")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short.f32le")
                .as_slice(),
            2,
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-before-tns.aac"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-before-tns.f32le"
            )
            .as_slice(),
            2,
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-after-tns.aac"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-after-tns.f32le"
            )
            .as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-after-tns-band-gain-signed.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-after-tns-band-gain-signed.f32le").as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-before-tns.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-before-tns.f32le").as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns.f32le").as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-after-tns-band-gain-signed-multiband.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-after-tns-band-gain-signed-multiband.f32le").as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns-band-gain-signed-multiband.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns-band-gain-signed-multiband.f32le").as_slice(),
            2,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns-band-gain-signed-multiband-groups.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns-band-gain-signed-multiband-groups.f32le").as_slice(),
            2,
        ),
    ] {
        let mut reader = StreamReader::open(Cursor::new(data)).unwrap();
        let mut decoder = NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
        assert_eq!(usize::from(decoder.channels()), channels);
        let mut packets = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            packets.push(p);
        }
        assert_eq!(packets.len(), 6);
        let mut actual = Vec::new();
        let mut state = None;
        for (i, p) in packets.iter().enumerate() {
            actual.extend(decoder.decode(p).unwrap());
            if i == 2 {
                state = Some(decoder.checkpoint());
            }
        }
        assert_eq!(actual.len() * 4, expected.len());
        assert!(
            actual.iter().any(|v| v.abs() > 0.00001),
            "silent target must receive nonzero coupling PCM"
        );
        let mut peak = 0f32;
        for (sample, bytes) in actual.iter().zip(expected.as_chunks::<4>().0.iter()) {
            peak = peak.max((sample - f32::from_le_bytes(*bytes)).abs());
        }
        assert!(
            peak < 0.0000001,
            "coupling PCM oracle mismatch: {peak}"
        );
        decoder.reset();
        decoder.restore(&state.unwrap()).unwrap();
        let tail: Vec<_> = packets[3..]
            .iter()
            .flat_map(|p| decoder.decode(p).unwrap())
            .collect();
        assert_eq!(tail, actual[3 * 1024 * channels..]);
        decoder.reset();
        let replay: Vec<_> = packets
            .iter()
            .flat_map(|p| decoder.decode(p).unwrap())
            .collect();
        assert_eq!(replay, actual);
    }
}

#[test]
fn absent_coupling_target_does_not_mutate_overlap_or_noise() {
    for (good, bad) in [
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-missing-target.aac")
                .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-before-tns.aac"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-missing-target-before-tns.aac"
            )
            .as_slice(),
        ),
    ] {
        let mut reader = StreamReader::open(Cursor::new(good)).unwrap();
        let mut decoder = NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
        let first = reader.next_packet().unwrap().unwrap();
        let second = reader.next_packet().unwrap().unwrap();
        decoder.decode(&first).unwrap();
        let state = decoder.checkpoint();
        let expected = decoder.decode(&second).unwrap();
        decoder.restore(&state).unwrap();
        let mut invalid = StreamReader::open(Cursor::new(bad)).unwrap();
        let packet = invalid.next_packet().unwrap().unwrap();
        assert!(
            decoder
                .decode(&packet)
                .unwrap_err()
                .to_string()
                .contains("coupling target is absent")
        );
        assert_eq!(decoder.decode(&second).unwrap(), expected);
    }
}

#[test]
fn independent_cpe_selection_routes_only_to_selected_channels() {
    for (data, selection) in [
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-shared.aac")
                .as_slice(),
            0,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-right.aac")
                .as_slice(),
            1,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-left.aac")
                .as_slice(),
            2,
        ),
    ] {
        let mut reader = StreamReader::open(Cursor::new(data)).unwrap();
        let mut decoder = NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
        let mut peak = [0f32; 2];
        while let Some(packet) = reader.next_packet().unwrap() {
            let pcm = decoder.decode(&packet).unwrap();
            for pair in pcm.as_chunks::<2>().0.iter() {
                for channel in 0..2 {
                    peak[channel] = peak[channel].max(pair[channel].abs());
                }
                if selection == 0 {
                    assert_eq!(pair[0], pair[1], "shared gain must produce equal channels");
                }
                if selection == 1 {
                    assert_eq!(pair[0], 0.0, "right selection must leave left silent");
                }
                if selection == 2 {
                    assert_eq!(pair[1], 0.0, "left selection must leave right silent");
                }
            }
        }
        if selection != 1 {
            assert!(peak[0] > 0.00001);
        }
        if selection != 2 {
            assert!(peak[1] > 0.00001);
        }
    }
}

#[test]
fn dependent_tns_fixtures_distinguish_coupling_stages() {
    for (before, after) in [
        (
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-before-tns.f32le"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-after-tns.f32le"
            )
            .as_slice(),
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-short-before-tns.f32le"
            )
            .as_slice(),
            include_bytes!(
                "fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns.f32le"
            )
            .as_slice(),
        ),
    ] {
        assert_eq!(before.len(), after.len());
        let difference = before
            .as_chunks::<4>().0.iter()
            .zip(after.as_chunks::<4>().0.iter())
            .map(|(a, b)| {
                (f32::from_le_bytes(*a)
                    - f32::from_le_bytes(*b))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(
            difference > 0.00001,
            "active target TNS must distinguish coupling stages: {difference}"
        );
    }
}

#[test]
fn multiband_fixture_accumulates_signed_gains_and_skips_zero_bands() {
    use fvid::codec::{
        aac_coupling::Coupling, aac_pair::ChannelPair, aac_pce::ProgramConfig, bits::BitReader,
        config::AacConfig,
    };
    for data in [
        include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-after-tns-band-gain-signed-multiband.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns-band-gain-signed-multiband.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo-short-after-tns-band-gain-signed-multiband-groups.aac").as_slice(),
    ] {
        let mut reader=StreamReader::open(Cursor::new(data)).unwrap();
        let config=AacConfig::parse(reader.audio_specific_config()).unwrap();
        while let Some(packet)=reader.next_packet().unwrap() {
            let mut bits=BitReader::new(&packet);
            assert_eq!(bits.read(3).unwrap(), 5);
            ProgramConfig::read(&mut bits, 0).unwrap();
            assert_eq!(bits.read(3).unwrap(), 1);
            assert_eq!(bits.read(4).unwrap(), 0);
            ChannelPair::read(&mut bits, &config).unwrap();
            assert_eq!(bits.read(3).unwrap(), 2);
            let cce=Coupling::read(&mut bits, &config).unwrap();
            assert_eq!(cce.point,1);
            let groups=cce.channel.info.group_lengths.len();
            assert_eq!(cce.channel.codebooks, vec![vec![1,0,1];groups]);
            assert_eq!(cce.targets[0].bands, vec![vec![1.0,0.0,1.0];groups]);
            let mut expected=vec![vec![-2f32.powf(-0.25),0.0,2f32.powf(-0.125)]];
            if groups == 2 {
                assert_eq!(cce.channel.info.group_lengths,vec![4,4]);
                expected.push(vec![2f32.powf(-0.125),0.0,-2f32.powf(-0.125)]);
            }
            assert_eq!(cce.targets[1].bands,expected);
            assert_eq!(bits.read(3).unwrap(),7);
        }
    }
}
