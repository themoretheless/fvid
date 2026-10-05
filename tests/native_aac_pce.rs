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
fn asc_preserves_program_and_reports_explicit_channel_count() {
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
    assert_eq!(AacConfig::parse(asc).unwrap().channels, 8);
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
        .as_chunks::<4>().0.iter()
        .map(|b| f32::from_le_bytes(*b))
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

#[test]
fn owned_cli_exports_pce_with_priming_and_explicit_wav_speakers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/fixtures/audio/aac-pce-wide8.m4a");
    let directory = std::env::temp_dir().join(format!("fvid-pce-export-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let output = directory.join("output.wav");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let wave = fvid::native_pcm::inspect(&mut std::fs::File::open(&output).unwrap(), None).unwrap();
    assert_eq!((wave.channels, wave.channel_mask), (8, 0xff));
    let raw = directory.join("output.f32le");
    fvid::native_export::export_audio_pcm_selected(
        &source, &raw, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    let pcm = std::fs::read(raw).unwrap();
    let reference = include_bytes!("fixtures/audio/aac-pce-wide8-export-reference.f32le");
    assert_eq!(pcm.len(), reference.len());
    for (a, b) in pcm.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
        let delta = (f32::from_le_bytes(*a)
            - f32::from_le_bytes(*b))
        .abs();
        assert!(delta < 1e-6, "PCM error {delta}");
    }
    {
        let api = directory.join("api.wav");
        let stats = fvid::media::decode_audio(&source, &api, &Default::default()).unwrap();
        assert_eq!(stats.channels, 8);
        assert_eq!(std::fs::read(api).unwrap(), std::fs::read(&output).unwrap());
    }
}

#[test]
fn adts_pce_stream_and_indexed_decode_preserve_all_samples_and_wav_layout() {
    use fvid::container::adts::{Aac, Limits, StreamReader};
    let input = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    let reference = include_bytes!("fixtures/audio/aac-pce-wide8-adts-reference.f32le");
    let indexed = Aac::parse(input, &Limits::default()).unwrap();
    assert_eq!(indexed.channels, 8);
    let reader = StreamReader::open(std::io::Cursor::new(input)).unwrap();
    assert_eq!(reader.configuration().channels, 8);
    let mut sequential = Vec::new();
    fvid::native_media::decode_adts_aac_reader(reader, &mut sequential, None).unwrap();
    let mut buffered = Vec::new();
    fvid::native_media::decode_aac_pcm(input, &mut buffered, &Limits::default()).unwrap();
    assert_eq!(buffered, sequential);
    assert_eq!(sequential.len(), reference.len());
    for (a, b) in sequential.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
        let delta = (f32::from_le_bytes(*a)
            - f32::from_le_bytes(*b))
        .abs();
        assert!(delta < 1e-6);
    }
    for cut in 7..indexed.frames[0].size {
        assert!(
            StreamReader::open(std::io::Cursor::new(&input[..cut])).is_err(),
            "first frame cut {cut}"
        );
    }
    let directory = std::env::temp_dir().join(format!("fvid-adts-pce-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-pce-wide8.aac");
    let output = directory.join("output.wav");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let wave = fvid::native_pcm::inspect(&mut std::fs::File::open(&output).unwrap(), None).unwrap();
    assert_eq!((wave.channels, wave.channel_mask), (8, 0xff));
    {
        let api = directory.join("api.wav");
        fvid::media::decode_audio(&source, &api, &Default::default()).unwrap();
        assert_eq!(std::fs::read(api).unwrap(), std::fs::read(output).unwrap());
    }
}

#[test]
fn pce_comment_uses_multibyte_descriptor_lengths_without_losing_layout() {
    let input = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    let mut bits = BitReader::new(&input[7..]);
    assert_eq!(bits.read(3).unwrap(), 5);
    let mut program = ProgramConfig::read(&mut bits, 0).unwrap();
    program.comment = vec![42; 255];
    let asc = program.audio_specific_config().unwrap();
    assert!(asc.len() > 127);
    let esds = fvid::container::adts::esds_for(&asc).unwrap();
    assert_eq!(
        fvid::codec::config::aac_specific_config(&esds).unwrap(),
        asc
    );
    let (_, parsed) = fvid::codec::config::AacConfig::parse_with_program(&asc).unwrap();
    assert_eq!(parsed.unwrap(), program);
}

#[test]
fn adts_pce_matroska_remux_and_concat_preserve_owned_pcm() {
    use fvid::container::{adts::StreamReader, matroska_write};
    let source = include_bytes!("fixtures/audio/aac-pce-wide8.aac").as_slice();
    let mut expected = Vec::new();
    fvid::native_media::decode_adts_aac_reader(
        StreamReader::open(source).unwrap(),
        &mut expected,
        None,
    )
    .unwrap();
    for segments in [1, 2] {
        let mut output = std::io::Cursor::new(Vec::new());
        if segments == 1 {
            matroska_write::write_adts(
                StreamReader::open(source).unwrap(),
                &mut output,
                None,
                None,
            )
            .unwrap();
        } else {
            matroska_write::concat_adts(
                vec![
                    StreamReader::open(source).unwrap(),
                    StreamReader::open(source).unwrap(),
                ],
                &mut output,
                None,
                None,
            )
            .unwrap();
        }
        let mut actual = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(output.get_ref(), &mut actual, None)
            .unwrap();
        let indexed = fvid::container::adts::Aac::parse(source, &Default::default()).unwrap();
        let mut decoder =
            fvid::codec::aac_native::NativeAacDecoder::new(&indexed.configuration).unwrap();
        let mut continuous = Vec::new();
        for _ in 0..segments {
            for packet in 0..indexed.packets() {
                for sample in decoder.decode(indexed.packet(packet)).unwrap() {
                    continuous.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
        assert_eq!(actual, continuous);
        if segments == 1 {
            assert_eq!(actual, expected);
        }
        assert_eq!(actual.len(), expected.len() * segments);
    }
}

#[test]
fn adts_concat_plan_compares_complete_program_configuration() {
    let directory = std::env::temp_dir().join(format!("fvid-pce-plan-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let first = directory.join("first.aac");
    let second = directory.join("second.aac");
    let source = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    std::fs::write(&first, source).unwrap();
    std::fs::write(&second, source).unwrap();
    assert!(fvid::native_plan::concat_adts(&[first.clone(), second.clone()]).is_ok());
    let mut changed = source.to_vec();
    changed[7] ^= 2; // PCE element_instance_tag, fixed ADTS header unchanged.
    std::fs::write(&second, changed).unwrap();
    assert!(
        fvid::native_plan::concat_adts(&[first, second])
            .unwrap_err()
            .contains("identical AAC configurations")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn adts_pce_mp4_writers_preserve_packets_layout_and_continuous_pcm() {
    use fvid::container::{adts, mp4::Mp4Reader, mp4_write};
    let source = include_bytes!("fixtures/audio/aac-pce-wide8.aac").as_slice();
    let indexed = adts::Aac::parse(source, &Default::default()).unwrap();
    for mode in 0..3 {
        let mut output = std::io::Cursor::new(Vec::new());
        let segments = if mode == 2 { 2 } else { 1 };
        match mode {
            0 => {
                mp4_write::write_adts_aac(source, &mut output).unwrap();
            }
            1 => {
                mp4_write::write_adts_aac_reader(
                    adts::StreamReader::open(source).unwrap(),
                    &mut output,
                )
                .unwrap();
            }
            _ => {
                mp4_write::concat_adts_readers(
                    vec![
                        adts::StreamReader::open(source).unwrap(),
                        adts::StreamReader::open(source).unwrap(),
                    ],
                    &mut output,
                    None,
                    None,
                )
                .unwrap();
            }
        }
        let mut reader =
            Mp4Reader::open(std::io::Cursor::new(output.get_ref()), Default::default()).unwrap();
        let track = &reader.tracks()[0];
        assert_eq!(track.channels, 8);
        assert_eq!(
            fvid::codec::config::aac_specific_config(&track.configuration).unwrap(),
            indexed.configuration
        );
        assert_eq!(track.samples.len(), indexed.packets() * segments);
        let mut packet = Vec::new();
        let mut expected = Vec::new();
        let mut decoder =
            fvid::codec::aac_native::NativeAacDecoder::new(&indexed.configuration).unwrap();
        for index in 0..indexed.packets() * segments {
            reader.read_packet(0, index, &mut packet).unwrap();
            assert_eq!(packet, indexed.packet(index % indexed.packets()));
            for sample in decoder.decode(&packet).unwrap() {
                expected.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(output.get_ref(), &mut actual).unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn standard_configuration_seven_matches_strict_independent_wide_pcm() {
    let source = include_bytes!("fixtures/audio/aac-config7-wide8.aac");
    let reference = include_bytes!("fixtures/audio/aac-config7-wide8-reference.f32le");
    let indexed = fvid::container::adts::Aac::parse(source, &Default::default()).unwrap();
    assert_eq!(indexed.channels, 8);
    let decoder = fvid::codec::aac_native::NativeAacDecoder::new(&indexed.configuration).unwrap();
    assert_eq!(decoder.channel_mask(), 0xff);
    let mut actual = Vec::new();
    fvid::native_media::decode_aac_pcm(source, &mut actual, &Default::default()).unwrap();
    assert_eq!(actual.len(), reference.len());
    for (a, b) in actual.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
        assert!(
            (f32::from_le_bytes(*a) - f32::from_le_bytes(*b))
                .abs()
                < 1e-6
        );
    }
    let mut output = Vec::new();
    fvid::container::mp4_write::write_adts_aac(source, &mut output).unwrap();
    let mut remuxed = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&output, &mut remuxed).unwrap();
    assert_eq!(actual, remuxed);
    let mut streamed = Vec::new();
    fvid::native_media::decode_adts_aac_reader(
        fvid::container::adts::StreamReader::open(source.as_slice()).unwrap(),
        &mut streamed,
        None,
    )
    .unwrap();
    assert_eq!(streamed, actual);
    let directory = std::env::temp_dir().join(format!("fvid-config7-wav-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.aac");
    let output = directory.join("output.wav");
    std::fs::write(&input, source).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let wav = fvid::native_pcm::inspect(&mut std::fs::File::open(output).unwrap(), None).unwrap();
    assert_eq!((wav.channels, wav.channel_mask), (8, 0xff));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn adts_pce_after_aligned_data_stream_preserves_pcm_and_checks_truncation() {
    use fvid::container::adts::{Aac, StreamReader};
    let source = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    let mut expected = Vec::new();
    fvid::native_media::decode_aac_pcm(source, &mut expected, &Default::default()).unwrap();
    for count in [0usize, 7, 255, 510] {
        // ID_DSE=4, tag=0, data_byte_align_flag=1, then count bytes.
        let mut prefix = vec![0x81, count.min(255) as u8];
        if count >= 255 {
            prefix.push((count - 255) as u8);
        }
        prefix.extend(vec![0x55; count]);
        let old_size = ((source[3] as usize & 3) << 11)
            | ((source[4] as usize) << 3)
            | (source[5] as usize >> 5);
        let size = old_size + prefix.len();
        let mut altered = source[..7].to_vec();
        altered[3] = (altered[3] & 252) | (size >> 11) as u8;
        altered[4] = (size >> 3) as u8;
        altered[5] = (altered[5] & 31) | ((size & 7) << 5) as u8;
        altered.extend(prefix);
        altered.extend_from_slice(&source[7..]);
        let mut actual = Vec::new();
        fvid::native_media::decode_adts_aac_reader(
            StreamReader::open(altered.as_slice()).unwrap(),
            &mut actual,
            None,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let indexed = Aac::parse(&altered, &Default::default()).unwrap();
        assert_eq!(indexed.channels, 8);
        for cut in 7..size {
            assert!(StreamReader::open(&altered[..cut]).is_err());
        }
    }
}

#[test]
fn adts_program_after_fill_elements_retains_audio_and_rejects_tools() {
    use fvid::container::adts::StreamReader;
    let source = include_bytes!("fixtures/audio/aac-pce-wide8.aac");
    let mut expected = Vec::new();
    fvid::native_media::decode_aac_pcm(source, &mut expected, &Default::default()).unwrap();
    for count in [0usize, 1, 14, 15, 269] {
        let mut bits = Vec::new();
        let mut field = |value: usize, width: usize| {
            for bit in (0..width).rev() {
                bits.push((value >> bit) & 1 != 0);
            }
        };
        // Eight FIL elements make the prefix byte aligned; the original PCE
        // alignment and audio payload are retained verbatim.
        for _ in 0..8 {
            field(6, 3);
            field(count.min(15), 4);
            if count >= 15 {
                field(count - 14, 8);
            }
            for index in 0..count {
                field(if count == 14 && index == 0 { 0x10 } else { 0 }, 8);
            }
        }
        let prefix: Vec<u8> = bits
            .as_chunks::<8>().0.iter()
            .map(|byte| byte.iter().fold(0, |value, bit| value * 2 + u8::from(*bit)))
            .collect();
        assert_eq!(bits.len() % 8, 0);
        let old_size = ((source[3] as usize & 3) << 11)
            | ((source[4] as usize) << 3)
            | (source[5] as usize >> 5);
        let size = old_size + prefix.len();
        let mut altered = source[..7].to_vec();
        altered[3] = (altered[3] & 252) | (size >> 11) as u8;
        altered[4] = (size >> 3) as u8;
        altered[5] = (altered[5] & 31) | ((size & 7) << 5) as u8;
        altered.extend(prefix);
        altered.extend_from_slice(&source[7..]);
        let mut actual = Vec::new();
        fvid::native_media::decode_adts_aac_reader(
            StreamReader::open(altered.as_slice()).unwrap(),
            &mut actual,
            None,
        )
        .unwrap();
        assert_eq!(actual, expected);
        for cut in 7..size {
            assert!(StreamReader::open(&altered[..cut]).is_err());
        }
        if count != 0 {
            let payload_bit = if count >= 15 { 15 } else { 7 };
            // Change the first extension to SBR (13), which must not be skipped.
            for (offset, set) in [true, true, false, true].into_iter().enumerate() {
                let bit = 56 + payload_bit + offset;
                if set {
                    altered[bit / 8] |= 1 << (7 - bit % 8);
                } else {
                    altered[bit / 8] &= !(1 << (7 - bit % 8));
                }
            }
            assert!(StreamReader::open(altered.as_slice()).is_err());
        }
    }
}

#[test]
fn indexed_61_and_71_layouts_match_independent_pcm_per_speaker() {
    for (source, reference, configuration, mask, channels) in [
        (
            include_bytes!("fixtures/audio/aac-config11-61.m4a").as_slice(),
            include_bytes!("fixtures/audio/aac-config11-61-reference.f32le").as_slice(),
            11,
            0x13f,
            7,
        ),
        (
            include_bytes!("fixtures/audio/aac-config12-71.m4a").as_slice(),
            include_bytes!("fixtures/audio/aac-config12-71-reference.f32le").as_slice(),
            12,
            0x63f,
            8,
        ),
    ] {
        let directory = std::env::temp_dir().join(format!(
            "fvid-layout-{configuration}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("input.m4a");
        let output = directory.join("output.wav");
        std::fs::write(&input, source).unwrap();
        fvid::native_export::export_audio_pcm_selected(
            &input, &output, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        let info =
            fvid::native_pcm::inspect(&mut std::fs::File::open(output).unwrap(), None).unwrap();
        assert_eq!(
            (info.channels, info.channel_mask),
            (u16::from(channels), mask)
        );
        std::fs::remove_dir_all(directory).unwrap();
        let mut reader =
            fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(source), Default::default())
                .unwrap();
        let track = reader.tracks()[0].clone();
        let asc = fvid::codec::config::aac_specific_config(&track.configuration).unwrap();
        let config = fvid::codec::config::AacConfig::parse(asc).unwrap();
        assert_eq!(
            (config.channel_configuration, config.channels),
            (configuration, channels)
        );
        let mut decoder = fvid::codec::aac_native::NativeAacDecoder::new(asc).unwrap();
        assert_eq!(decoder.channel_mask(), mask);
        let mut actual = Vec::new();
        let mut packet = Vec::new();
        for index in 0..track.samples.len() {
            reader.read_packet(0, index, &mut packet).unwrap();
            actual.extend(decoder.decode(&packet).unwrap());
        }
        // The reference retains the last MP4 packet's shorter duration.
        let last = track.samples.get(track.samples.len() - 1).unwrap();
        assert_eq!(
            actual.len() * 4 - reference.len(),
            (1024 - last.duration as usize) * usize::from(channels) * 4
        );
        for (a, b) in actual.iter().zip(reference.as_chunks::<4>().0.iter()) {
            assert!((*a - f32::from_le_bytes(*b)).abs() < 1e-6);
        }
    }
}
