use fvid::container::{adts, mp4, mp4_write};
#[test]
fn adts_to_mp4_preserves_packets_clock_and_owned_pcm() {
    for data in [
        include_bytes!("fixtures/audio/aac-stereo.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-mono-44k.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-51-active.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-96k.aac").as_slice(),
        include_bytes!("fixtures/audio/aac-88k.aac").as_slice(),
    ] {
        let source = adts::Aac::parse(data, &Default::default()).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            mp4_write::write_adts_aac(data, &mut output).unwrap(),
            source.packets() as u64
        );
        let mut streaming = std::io::Cursor::new(Vec::new());
        let input = adts::StreamReader::open(data).unwrap();
        assert_eq!(
            mp4_write::write_adts_aac_reader(input, &mut streaming).unwrap(),
            source.packets() as u64
        );
        let mut stream_pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(streaming.get_ref(), &mut stream_pcm).unwrap();
        let mut reader =
            mp4::Mp4Reader::open(std::io::Cursor::new(&output), Default::default()).unwrap();
        let track = &reader.tracks()[0];
        assert_eq!(
            (track.sample_rate, track.channels, track.duration),
            (source.sample_rate, source.channels, source.samples())
        );
        assert!(track.edits.is_empty());
        let mut packet = Vec::new();
        for index in 0..source.packets() {
            assert_eq!(
                reader.tracks()[0].samples.get(index).unwrap().pts,
                index as i64 * 1024
            );
            reader.read_packet(0, index, &mut packet).unwrap();
            assert_eq!(packet, source.packet(index));
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        fvid::native_media::decode_aac_pcm(data, &mut before, &Default::default()).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&output, &mut after).unwrap();
        assert_eq!(before, after);
        assert_eq!(stream_pcm, before);
        let mut streamed = mp4::Mp4Reader::open(streaming, Default::default()).unwrap();
        for index in 0..source.packets() {
            streamed.read_packet(0, index, &mut packet).unwrap();
            assert_eq!(packet, source.packet(index));
        }
    }
}

#[test]
fn remux_cli_never_overwrites_and_rejects_truncated_tail() {
    let dir = std::env::temp_dir().join(format!("fvid-remux-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.aac");
    let output = dir.join("out.m4a");
    let fixture = include_bytes!("fixtures/audio/aac-stereo.aac");
    std::fs::write(&source, fixture).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let original = std::fs::read(&output).unwrap();
    assert!(fvid::native_export::remux_adts_aac(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    std::fs::write(&source, &fixture[..fixture.len() - 1]).unwrap();
    let failed = dir.join("failed.m4a");
    assert!(fvid::native_export::remux_adts_aac(&source, &failed).is_err());
    assert!(!failed.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn extended_audio_entry_rejects_invalid_rates_and_channels() {
    let mut bytes = Vec::new();
    mp4_write::write_adts_aac(include_bytes!("fixtures/audio/aac-96k.aac"), &mut bytes).unwrap();
    let entry = bytes.windows(4).rposition(|s| s == b"mp4a").unwrap() + 4;
    assert_eq!(&bytes[entry + 8..entry + 10], &2u16.to_be_bytes());
    assert_eq!(
        f64::from_be_bytes(bytes[entry + 32..entry + 40].try_into().unwrap()),
        96000.0
    );
    for rate in [f64::NAN, f64::INFINITY, -1.0, 0.0, 96000.5] {
        let mut invalid = bytes.clone();
        invalid[entry + 32..entry + 40].copy_from_slice(&rate.to_be_bytes());
        assert!(mp4::Mp4Reader::open(std::io::Cursor::new(invalid), Default::default()).is_err());
    }
    for channels in [0u32, 65536] {
        let mut invalid = bytes.clone();
        invalid[entry + 40..entry + 44].copy_from_slice(&channels.to_be_bytes());
        assert!(mp4::Mp4Reader::open(std::io::Cursor::new(invalid), Default::default()).is_err());
    }
}

#[test]
fn streaming_remux_writes_extended_mdat_beyond_four_gibibytes() {
    use std::io::{Read, Seek, SeekFrom, Write};
    // Framing-only SSR-profile packets have no implicit SBR discovery.
    // The muxer copies these dummy payloads without decoding them.
    // Repetition and sparse output exercise >4 GiB without allocating that data.
    struct Repeat {
        frame: Vec<u8>,
        position: u64,
        end: u64,
    }
    impl Read for Repeat {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let at = self.position as usize % self.frame.len();
            let n = out
                .len()
                .min(self.frame.len() - at)
                .min((self.end - self.position) as usize);
            out[..n].copy_from_slice(&self.frame[at..at + n]);
            self.position += n as u64;
            Ok(n)
        }
    }
    struct Sparse(std::fs::File);
    impl Write for Sparse {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() == 8184 && bytes.iter().all(|b| *b == 0) {
                self.0.seek(SeekFrom::Current(bytes.len() as i64))?;
                Ok(bytes.len())
            } else {
                self.0.write(bytes)
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    impl Seek for Sparse {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(pos)
        }
    }
    let path = std::env::temp_dir().join(format!("fvid-remux-64-{}.mp4", std::process::id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    let mut frame = vec![0; 8191];
    frame[..7].copy_from_slice(&include_bytes!("fixtures/audio/aac-mono-44k.aac")[..7]);
    frame[2] = (frame[2] & 0x3f) | 0x80; // SSR framing; this test does not decode dummy PCM.
    frame[3] = (frame[3] & !3) | 3;
    frame[4] = 255;
    frame[5] |= 224;
    let count = 530_000u64;
    let input = adts::StreamReader::open(Repeat {
        end: count * frame.len() as u64,
        frame,
        position: 0,
    })
    .unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let mut output = Sparse(file);
    assert_eq!(
        mp4_write::write_adts_aac_reader(input, &mut output).unwrap(),
        count
    );
    assert!(output.stream_position().unwrap() > u64::from(u32::MAX));
    drop(output);
    let mut reader =
        mp4::Mp4Reader::open(std::fs::File::open(&path).unwrap(), Default::default()).unwrap();
    let track = &reader.tracks()[0];
    assert_eq!(track.samples.len(), count as usize);
    assert_eq!(track.duration, count * 1024);
    assert!(track.samples.get(count as usize - 1).unwrap().offset > u64::from(u32::MAX));
    let mut packet = Vec::new();
    reader
        .read_packet(0, count as usize - 1, &mut packet)
        .unwrap();
    assert_eq!(packet, vec![0; 8184]);
}
