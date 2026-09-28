use fvid::{container::adts::Limits, native_media::decode_aac_pcm};

#[test]
fn headless_aac_export_matches_saved_pcm_reference() {
    let mut pcm = Vec::new();
    let stats = decode_aac_pcm(include_bytes!("fixtures/audio/aac-mono-44k.aac"),
        &mut pcm, &Limits::default()).unwrap();
    assert_eq!((stats.sample_rate, stats.channels, stats.decoded_frames, stats.sample_frames),
        (44100, 1, 7, 7168));
    let oracle = include_bytes!("fixtures/audio/aac-mono-reference.f32le");
    assert_eq!(pcm.len(), oracle.len());
    let mut squared = 0.0f64;
    let mut peak = 0.0f64;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        let delta = f64::from(f32::from_le_bytes(a.try_into().unwrap()))
            - f64::from(f32::from_le_bytes(b.try_into().unwrap()));
        squared += delta * delta;
        peak = peak.max(delta.abs());
    }
    assert!((squared / stats.sample_frames as f64).sqrt() < 4e-5);
    assert!(peak < 3e-4);
}

#[test]
fn destination_errors_are_propagated() {
    struct Fails;
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("destination failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let error = decode_aac_pcm(include_bytes!("fixtures/audio/aac-mono-44k.aac"),
        &mut Fails, &Limits::default()).unwrap_err();
    assert!(error.to_string().contains("destination failed"));
}

#[test]
fn cli_exports_complete_pcm_without_overwriting_or_partial_files() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(dir.clone());
    let source = dir.join("source.aac");
    let output = dir.join("out.f32le");
    let fixture = include_bytes!("fixtures/audio/aac-mono-44k.aac");
    std::fs::write(&source, fixture).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&output).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"sample_frames\":7168"));
    let mut expected = Vec::new();
    decode_aac_pcm(fixture, &mut expected, &Limits::default()).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    assert!(fvid::native_export::export_aac_pcm(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    let frames = fvid::container::adts::Aac::parse(fixture, &Limits::default()).unwrap();
    let second = frames.frames[1];
    let mut corrupt = fixture.to_vec();
    corrupt[second.start + second.header_bytes..second.start + second.size].fill(0xff);
    std::fs::write(&source, corrupt).unwrap();
    let failed = dir.join("failed.f32le");
    assert!(fvid::native_export::export_aac_pcm(&source, &failed).is_err());
    assert!(!failed.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
}

#[test]
fn float_wav_preserves_surround_samples_and_channel_layout() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-wav-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.aac");
    let output = dir.join("out.wav");
    let input = include_bytes!("fixtures/audio/aac-51-active.aac");
    std::fs::write(&source, input).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&output).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let wave = std::fs::read(&output).unwrap();
    let read32 = |at| u32::from_le_bytes(wave[at..at+4].try_into().unwrap());
    assert_eq!(&wave[..4], b"RIFF");
    assert_eq!(read32(4) as usize + 8, wave.len());
    assert_eq!(&wave[8..16], b"WAVEfmt ");
    assert_eq!(read32(16), 40);
    assert_eq!(&wave[20..24], &[0xfe, 0xff, 6, 0]);
    assert_eq!(read32(24), 48000);
    assert_eq!(read32(28), 48000 * 24);
    assert_eq!(read32(40), 0x3f);
    assert_eq!(&wave[44..60], &[3,0,0,0,0,0,16,0,128,0,0,170,0,56,155,113]);
    assert_eq!(&wave[60..64], b"fact");
    assert_eq!(&wave[72..76], b"data");
    let mut pcm = Vec::new();
    let stats = decode_aac_pcm(input, &mut pcm, &Limits::default()).unwrap();
    assert_eq!(u64::from(read32(68)), stats.sample_frames);
    assert_eq!(read32(76) as usize, pcm.len());
    assert_eq!(&wave[80..], pcm);
    assert!(fvid::native_export::export_aac_pcm(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), wave);
    let clip = dir.join("clip.wav");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&clip)
        .args(["--from", "0.030001", "--to", "0.070001", "--quiet"])
        .output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(run.stdout.is_empty());
    let clipped = std::fs::read(clip).unwrap();
    assert_eq!(u32::from_le_bytes(clipped[68..72].try_into().unwrap()), 1920);
    assert_eq!(&clipped[80..], &pcm[1441*24..3361*24]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn interval_pcm_equals_exact_slice_including_decoder_preroll() {
    use std::time::Duration;
    let data = include_bytes!("fixtures/audio/aac-stereo.aac");
    let mut whole = Vec::new();
    decode_aac_pcm(data, &mut whole, &Limits::default()).unwrap();
    let mut part = Vec::new();
    let stats = fvid::native_media::decode_aac_pcm_interval(data, &mut part, &Limits::default(),
        Some((Duration::from_micros(30001), Duration::from_micros(70001)))).unwrap();
    // ceil(30001*48000/1e6)=1441; ceil(70001*48000/1e6)=3361.
    assert_eq!(stats.sample_frames, 1920);
    assert_eq!(stats.decoded_frames, 4);
    assert_eq!(part, whole[1441*8..3361*8]);
    for (from, to) in [(2, 3), (1, 1), (2, 1)] {
        let mut out = Vec::new();
        assert!(fvid::native_media::decode_aac_pcm_interval(data, &mut out, &Limits::default(),
            Some((Duration::from_secs(from), Duration::from_secs(to)))).is_err());
        assert!(out.is_empty());
    }
}

#[test]
fn mp4_aac_edit_removes_priming_and_encoder_tail() {
    let source = include_bytes!("fixtures/audio/aac-native-edit.m4a");
    let mut pcm = Vec::new();
    let stats = fvid::native_media::decode_mp4_aac_pcm(source, &mut pcm).unwrap();
    assert_eq!((stats.sample_rate, stats.channels, stats.sample_frames), (44100, 1, 5645));
    let oracle = include_bytes!("fixtures/audio/aac-native-edit-reference.f32le");
    assert!(oracle.len() >= pcm.len());
    let mut peak = 0.0f32;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        peak = peak.max((f32::from_le_bytes(a.try_into().unwrap())
            - f32::from_le_bytes(b.try_into().unwrap())).abs());
    }
    assert!(peak < 1e-6, "peak error {peak}");
}

#[test]
fn mp4_interval_is_relative_to_edited_audio_and_clips_at_tail() {
    use std::time::Duration;
    let source = include_bytes!("fixtures/audio/aac-native-edit.m4a");
    let mut whole = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(source, &mut whole).unwrap();
    for (from, to, first, last) in [(30001, 70001, 1324, 3088), (120000, 200000, 5292, 5645)] {
        let mut part = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm_interval(source, &mut part,
            Some((Duration::from_micros(from), Duration::from_micros(to)))).unwrap();
        assert_eq!(part, whole[first*4..last*4]);
        assert_eq!(stats.sample_frames as usize, last-first);
    }
    for (from, to) in [(2, 3), (1, 1), (2, 1)] {
        let mut out = Vec::new();
        assert!(fvid::native_media::decode_mp4_aac_pcm_interval(source, &mut out,
            Some((Duration::from_secs(from), Duration::from_secs(to)))).is_err());
        assert!(out.is_empty());
    }
}

#[test]
fn mp4_clock_rescaling_preserves_pcm_and_edit_selection() {
    use std::time::Duration;
    let original = include_bytes!("fixtures/audio/aac-native-edit.m4a");
    let mut rescaled = original.to_vec();
    let at = |tag: &[u8]| original.windows(4).position(|v| v == tag).unwrap();
    let double = |bytes: &mut [u8], offset: usize| {
        let value = u32::from_be_bytes(bytes[offset..offset+4].try_into().unwrap());
        bytes[offset..offset+4].copy_from_slice(&(value * 2).to_be_bytes());
    };
    let mdhd = at(b"mdhd");
    assert_eq!(original[mdhd+4], 0);
    double(&mut rescaled, mdhd+16); // timescale
    double(&mut rescaled, mdhd+20); // track duration
    let stts = at(b"stts");
    let entries = u32::from_be_bytes(original[stts+8..stts+12].try_into().unwrap());
    for entry in 0..entries as usize { double(&mut rescaled, stts+16+entry*8); }
    let elst = at(b"elst");
    assert_eq!(original[elst+4], 0);
    double(&mut rescaled, elst+16); // media_time; movie duration stays unchanged
    for interval in [None, Some((Duration::from_micros(30001), Duration::from_micros(70001)))] {
        let mut expected = Vec::new();
        let mut actual = Vec::new();
        let a = fvid::native_media::decode_mp4_aac_pcm_interval(original, &mut expected, interval).unwrap();
        let b = fvid::native_media::decode_mp4_aac_pcm_interval(&rescaled, &mut actual, interval).unwrap();
        assert_eq!(a, b);
        assert_eq!(actual, expected);
    }
    // A one-tick shift at twice the sample clock would be half a sample.
    let value = u32::from_be_bytes(rescaled[elst+16..elst+20].try_into().unwrap());
    rescaled[elst+16..elst+20].copy_from_slice(&(value+1).to_be_bytes());
    let error = fvid::native_media::decode_mp4_aac_pcm(&rescaled, &mut Vec::new()).unwrap_err();
    assert!(error.to_string().contains("not aligned"));
}

#[test]
fn mp4_edit_schedule_repeats_ranges_and_inserts_silence() {
    use std::time::Duration;
    let original = include_bytes!("fixtures/audio/aac-native-edit.m4a");
    let mut edited = original.to_vec();
    let at = |tag: &[u8]| original.windows(4).position(|v| v == tag).unwrap();
    let movie = at(b"mvhd");
    assert_eq!(u32::from_be_bytes(original[movie+16..movie+20].try_into().unwrap()), 44100);
    let elst = at(b"elst");
    let entries = [(1000u32, 1024i32), (441, -1), (500, 2500), (500, 1024)];
    let mut body = Vec::new();
    for (duration, start) in entries {
        body.extend_from_slice(&duration.to_be_bytes());
        body.extend_from_slice(&start.to_be_bytes());
        body.extend_from_slice(&0x10000u32.to_be_bytes());
    }
    edited[elst+8..elst+12].copy_from_slice(&4u32.to_be_bytes());
    // moov follows mdat, so expanding metadata cannot move packet offsets.
    assert!(at(b"moov") > at(b"mdat"));
    for tag in [b"moov", b"trak", b"edts", b"elst"] {
        let size_at = at(tag)-4;
        let size = u32::from_be_bytes(edited[size_at..size_at+4].try_into().unwrap());
        edited[size_at..size_at+4].copy_from_slice(&(size+36).to_be_bytes());
    }
    edited.splice(elst+12..elst+24, body);
    let mut whole = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(original, &mut whole).unwrap();
    let mut expected = whole[..1000*4].to_vec();
    expected.extend_from_slice(&vec![0; 441*4]);
    expected.extend_from_slice(&whole[1476*4..1976*4]);
    expected.extend_from_slice(&whole[..500*4]);
    let mut actual = Vec::new();
    let stats = fvid::native_media::decode_mp4_aac_pcm(&edited, &mut actual).unwrap();
    assert_eq!(stats.sample_frames, 2441);
    assert_eq!(actual, expected);
    let mut part = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm_interval(&edited, &mut part,
        Some((Duration::from_millis(10), Duration::from_millis(50)))).unwrap();
    assert_eq!(part, expected[441*4..2205*4]);
    // A requested source segment beyond the actual audio must fail.
    edited[elst+16..elst+20].copy_from_slice(&100000i32.to_be_bytes());
    assert!(fvid::native_media::decode_mp4_aac_pcm(&edited, &mut Vec::new()).is_err());
}

#[test]
fn matroska_aac_matches_saved_reference_and_interval() {
    use std::time::Duration;
    let data = include_bytes!("fixtures/audio/aac-stereo.mka");
    let mut pcm = Vec::new();
    let stats = fvid::native_media::decode_matroska_aac_pcm_interval(data, &mut pcm, None).unwrap();
    assert_eq!((stats.sample_rate, stats.channels), (48000, 2));
    let oracle = include_bytes!("fixtures/audio/aac-matroska-reference.f32le");
    assert_eq!(pcm.len(), oracle.len());
    let mut squared = 0.0f64;
    let mut peak = 0.0f64;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        let delta = f64::from(f32::from_le_bytes(a.try_into().unwrap()))
            - f64::from(f32::from_le_bytes(b.try_into().unwrap()));
        squared += delta*delta; peak = peak.max(delta.abs());
    }
    assert!((squared/(pcm.len()/4) as f64).sqrt() < 0.00015);
    assert!(peak < 0.003, "peak {peak}");
    let mut part = Vec::new();
    fvid::native_media::decode_matroska_aac_pcm_interval(data, &mut part,
        Some((Duration::from_micros(30001), Duration::from_micros(70001)))).unwrap();
    assert_eq!(part, pcm[1441*8..3361*8]);
}

#[test]
fn matroska_delay_and_both_padding_directions_trim_exact_samples() {
    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        let size = (body.len() as u32 | 0x1000_0000).to_be_bytes();
        [id, &size, body].concat()
    }
    let adts = include_bytes!("fixtures/audio/aac-stereo.aac");
    let stream = fvid::container::adts::Aac::parse(adts, &Limits::default()).unwrap();
    let mut whole = Vec::new();
    decode_aac_pcm(adts, &mut whole, &Limits::default()).unwrap();
    let header = element(&[0x1a,0x45,0xdf,0xa3], &element(&[0x42,0x82], b"matroska"));
    let info = element(&[0x15,0x49,0xa9,0x66], &element(&[0x2a,0xd7,0xb1], &1_000_000u32.to_be_bytes()));
    let audio = element(&[0xe1], &[
        element(&[0xb5], &48000f64.to_be_bytes()), element(&[0x9f], &[2]),
    ].concat());
    let track = element(&[0xae], &[
        element(&[0xd7], &[1]), element(&[0x83], &[2]), element(&[0x86], b"A_AAC"),
        element(&[0x63,0xa2], &stream.frames[0].asc),
        element(&[0x56,0xaa], &2_083_333u32.to_be_bytes()), audio,
    ].concat());
    let tracks = element(&[0x16,0x54,0xae,0x6b], &track);
    for padding in [1_041_666i64, -1_041_666] {
        let first = element(&[0xa3], &[&[0x81,0,0,0x80][..], stream.packet(0)].concat());
        let second = element(&[0xa0], &[
            element(&[0xa1], &[&[0x81,0,21,0][..], stream.packet(1)].concat()),
            element(&[0x75,0xa2], &padding.to_be_bytes()),
        ].concat());
        let cluster = element(&[0x1f,0x43,0xb6,0x75], &[
            element(&[0xe7], &[0]), first, second,
        ].concat());
        let data = [header.clone(), element(&[0x18,0x53,0x80,0x67],
            &[info.clone(), tracks.clone(), cluster].concat())].concat();
        let mut actual = Vec::new();
        let stats = fvid::native_media::decode_matroska_aac_pcm_interval(&data, &mut actual, None).unwrap();
        let expected = if padding > 0 { whole[100*8..1998*8].to_vec() }
            else { [&whole[100*8..1024*8], &whole[1074*8..2048*8]].concat() };
        assert_eq!(stats.sample_frames, 1898);
        assert_eq!(actual, expected);
    }
}

#[test]
fn native_volume_scales_all_aac_containers_without_clipping() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-gain-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    for (index, name) in ["aac-stereo.aac", "aac-native-edit.m4a", "aac-stereo.mka"].iter().enumerate() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio").join(name);
        let base = dir.join(format!("base{index}.f32le"));
        let baseline = fvid::native_export::export_aac_pcm(&source, &base).unwrap();
        let pcm = std::fs::read(base).unwrap();
        for gain in [0.0f32, 0.5, 64.0] {
            let output = dir.join(format!("gain{index}-{gain}.wav"));
            let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "decode-audio"]).arg(&source).arg(&output)
                .args(["--volume", &gain.to_string()]).output().unwrap();
            assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
            let wave = std::fs::read(&output).unwrap();
            assert_eq!(u32::from_le_bytes(wave[68..72].try_into().unwrap()) as u64, baseline.sample_frames);
            let expected: Vec<_> = pcm.chunks_exact(4).flat_map(|bytes|
                (f32::from_le_bytes(bytes.try_into().unwrap()) * gain).to_le_bytes()).collect();
            assert_eq!(&wave[80..], expected);
        }
        for gain in [-1.0, 65.0, f64::NAN, f64::INFINITY] {
            let output = dir.join("invalid.wav");
            assert!(fvid::native_export::export_aac_pcm_with_volume(&source, &output, None, gain).is_err());
            assert!(!output.exists());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
