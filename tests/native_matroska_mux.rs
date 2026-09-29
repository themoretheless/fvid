use fvid::container::{adts, matroska_write, webm};
use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-mka-mux-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
#[test]
fn packets_configuration_nanosecond_clock_and_pcm_survive_matroska() {
    let d = dir("roundtrip");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
        "aac-88k.aac",
    ] {
        let path = fixture(name);
        let data = std::fs::read(&path).unwrap();
        let source = adts::Aac::parse(&data, &Default::default()).unwrap();
        let mut output = std::io::Cursor::new(Vec::new());
        let event = matroska_write::write_adts(
            adts::StreamReader::open(data.as_slice()).unwrap(),
            &mut output,
            None,
            None,
        )
        .unwrap();
        assert!(!event.done);
        assert_eq!(event.packets, source.packets() as u64);
        let mut reader =
            webm::WebmReader::open(std::io::Cursor::new(output.get_ref()), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks.len(), 1);
        assert_eq!(reader.tracks[0].codec, "A_AAC");
        assert_eq!(reader.tracks[0].sample_rate, u64::from(source.sample_rate));
        assert_eq!(reader.tracks[0].channels, u64::from(source.channels));
        assert_eq!(reader.packets.len(), source.packets());
        for i in 0..source.packets() {
            assert_eq!(reader.read_packet(i).unwrap(), source.packet(i));
            assert_eq!(
                reader.packets[i].pts_ns,
                (i as u128 * 1024 * 1000000000 / u128::from(source.sample_rate)) as i64
            );
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        fvid::native_media::decode_aac_pcm(&data, &mut before, &Default::default()).unwrap();
        fvid::native_media::decode_matroska_aac_pcm_interval(output.get_ref(), &mut after, None)
            .unwrap();
        assert!(before == after, "PCM differs for {name}");
        let dest = d.0.join(format!("{name}.mka"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "remux"])
            .arg(&path)
            .arg(&dest)
            .arg("--progress")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(&dest).unwrap(), *output.get_ref());
        #[cfg(feature = "media")]
        {
            let api = d.0.join(format!("api-{name}.mkv"));
            let stats = fvid::media::remux(&path, &api, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(api).unwrap(), *output.get_ref());
        }
    }
}
#[test]
fn writer_errors_cancel_and_publication_never_leave_outputs() {
    let d = dir("errors");
    let src = d.0.join("source.aac");
    let dest = d.0.join("out.mka");
    let mut data = std::fs::read(fixture("aac-mono-44k.aac")).unwrap();
    std::fs::write(&src, &data).unwrap();
    let flag = fvid::media_control::CancelFlag::default();
    let stop = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets > 0 {
            stop.cancel();
        }
    });
    assert!(
        fvid::native_export::remux_adts_aac_controlled(&src, &dest, Some(&flag), Some(&hook))
            .is_err()
    );
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    data.pop();
    std::fs::write(&src, &data).unwrap();
    assert!(fvid::native_export::remux_adts_aac(&src, &dest).is_err());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    std::fs::copy(fixture("aac-mono-44k.aac"), &src).unwrap();
    std::fs::write(&dest, b"keep").unwrap();
    assert!(fvid::native_export::remux_adts_aac(&src, &dest).is_err());
    assert_eq!(std::fs::read(dest).unwrap(), b"keep");
    struct Fails(std::io::Cursor<Vec<u8>>);
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("output failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl std::io::Seek for Fails {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.0, p)
        }
    }
    assert!(
        matroska_write::write_adts(
            adts::StreamReader::open(std::fs::read(src).unwrap().as_slice()).unwrap(),
            &mut Fails(std::io::Cursor::new(Vec::new())),
            None,
            None
        )
        .is_err()
    );
}

#[test]
fn matroska_concat_joins_independent_segments_with_contiguous_clock() {
    let d = dir("concat");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
        "aac-88k.aac",
    ] {
        let original = std::fs::read(fixture(name)).unwrap();
        let split = adts::header(&original).unwrap().frame_bytes;
        let a = d.0.join(format!("{name}-a.aac"));
        let b = d.0.join(format!("{name}-b.aac"));
        std::fs::write(&a, &original[..split]).unwrap();
        std::fs::write(&b, &original[split..]).unwrap();
        let sources = vec![a, b];
        let output = d.0.join(format!("{name}.mka"));
        let stats = fvid::native_export::concat_adts_aac(&sources, &output, None, None).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        let source = adts::Aac::parse(&original, &Default::default()).unwrap();
        assert_eq!(stats.packets, source.packets() as u64);
        assert!(stats.done);
        let mut reader =
            webm::WebmReader::open(std::io::Cursor::new(&bytes), Default::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), source.packets());
        for i in 0..source.packets() {
            assert_eq!(reader.read_packet(i).unwrap(), source.packet(i));
            assert_eq!(
                reader.packets[i].pts_ns,
                (i as u128 * 1024 * 1000000000 / u128::from(source.sample_rate)) as i64
            );
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        fvid::native_media::decode_aac_pcm(&original, &mut before, &Default::default()).unwrap();
        fvid::native_media::decode_matroska_aac_pcm_interval(&bytes, &mut after, None).unwrap();
        assert!(before == after, "PCM mismatch for {name}");
        let cli = d.0.join(format!("cli-{name}.mkv"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "concat"])
            .arg(&cli)
            .args(&sources)
            .arg("--progress")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), bytes);
        #[cfg(feature = "media")]
        {
            let api = d.0.join(format!("api-{name}.mka"));
            let stats = fvid::media::concat(&sources, &api, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.segments, 2);
            assert_eq!(std::fs::read(api).unwrap(), bytes);
        }
        assert!(fvid::native_export::concat_adts_aac(&sources, &output, None, None).is_err());
        assert_eq!(std::fs::read(output).unwrap(), bytes);
    }
}

#[test]
fn sequence_failure_is_terminal_and_matroska_concat_discards_partial_output() {
    let d = dir("concat-errors");
    let data = std::fs::read(fixture("aac-mono-44k.aac")).unwrap();
    let split = adts::header(&data).unwrap().frame_bytes;
    let a = d.0.join("a.aac");
    let b = d.0.join("b.aac");
    let out = d.0.join("out.mka");
    std::fs::write(&a, &data[..split - 1]).unwrap();
    std::fs::write(&b, &data[split..]).unwrap();
    let mut sequence = adts::SequenceReader::new(vec![
        adts::StreamReader::open(&data[..split - 1]).unwrap(),
        adts::StreamReader::open(&data[split..]).unwrap(),
    ])
    .unwrap();
    assert!(sequence.next_packet().is_err());
    assert!(
        sequence
            .next_packet()
            .unwrap_err()
            .to_string()
            .contains("sequence failed")
    );
    let sources = vec![a.clone(), b.clone()];
    assert!(fvid::native_export::concat_adts_aac(&sources, &out, None, None).is_err());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 2);
    std::fs::write(&a, &data[..split]).unwrap();
    let flag = fvid::media_control::CancelFlag::default();
    let stop = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets > 0 {
            stop.cancel();
        }
    });
    assert!(
        fvid::native_export::concat_adts_aac(&sources, &out, Some(&flag), Some(&hook)).is_err()
    );
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 2);
    std::fs::copy(fixture("aac-stereo.aac"), &b).unwrap();
    assert!(fvid::native_export::concat_adts_aac(&sources, &out, None, None).is_err());
    assert!(!out.exists());
}

#[test]
fn codec_delay_and_signed_padding_preserve_audible_aac_samples() {
    use matroska_write::{Encoding, PacketWriter, TrackOptions, TrackSpec};
    let data = std::fs::read(fixture("aac-mono-44k.aac")).unwrap();
    let source = adts::Aac::parse(&data, &Default::default()).unwrap();
    let ns = |samples: u64| {
        (samples * 1_000_000_000 + u64::from(source.sample_rate) / 2)
            / u64::from(source.sample_rate)
    };
    let mut original = Vec::new();
    fvid::native_media::decode_aac_pcm(&data, &mut original, &Default::default()).unwrap();
    for (delay, head, tail) in [(128, 0, 256), (0, 128, 256)] {
        let mut output = std::io::Cursor::new(Vec::new());
        let mut writer = PacketWriter::new_with_options(
            &mut output,
            &[TrackSpec {
                encoding: Encoding::Aac {
                    configuration: &source.frames[0].asc,
                    sample_rate: source.sample_rate,
                    channels: source.channels,
                },
                name: "",
                language: "und",
            }],
            &[TrackOptions {
                codec_delay_ns: ns(delay),
                ..Default::default()
            }],
        )
        .unwrap();
        for i in 0..source.packets() {
            let padding = if i == 0 {
                -(ns(head) as i64)
            } else if i + 1 == source.packets() {
                ns(tail) as i64
            } else {
                0
            };
            writer
                .write_packet_with_padding(
                    0,
                    ns(i as u64 * 1024),
                    ns((i as u64 + 1) * 1024) - ns(i as u64 * 1024),
                    true,
                    source.packet(i),
                    padding,
                )
                .unwrap();
        }
        writer.finish().unwrap();
        let mut reader =
            webm::WebmReader::open(std::io::Cursor::new(output.get_ref()), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].codec_delay_ns, ns(delay));
        assert_eq!(reader.packets[0].discard_padding_ns, -(ns(head) as i64));
        assert_eq!(
            reader.packets.last().unwrap().discard_padding_ns,
            ns(tail) as i64
        );
        assert_eq!(
            reader.duration_ns,
            Some(ns(source.packets() as u64 * 1024) - ns(delay) - ns(tail))
        );
        let mut decoded = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(output.get_ref(), &mut decoded, None)
            .unwrap();
        let stride = usize::from(source.channels) * 4;
        assert_eq!(
            decoded,
            original[(delay.max(head) as usize * stride)..original.len() - tail as usize * stride]
        );
    }
}

#[test]
fn excessive_padding_poisoning_and_delay_overflow_are_rejected() {
    use matroska_write::{Encoding, PacketWriter, TrackOptions, TrackSpec};
    let tracks = [TrackSpec {
        encoding: Encoding::Aac {
            configuration: &[0x12, 0x08],
            sample_rate: 44100,
            channels: 1,
        },
        name: "",
        language: "und",
    }];
    let mut output = std::io::Cursor::new(Vec::new());
    assert!(
        PacketWriter::new_with_options(
            &mut output,
            &tracks,
            &[TrackOptions {
                codec_delay_ns: u64::MAX,
                ..Default::default()
            }]
        )
        .is_err()
    );
    assert!(output.get_ref().is_empty());
    for padding in [11, -11, i64::MIN] {
        let mut output = std::io::Cursor::new(Vec::new());
        let mut writer = PacketWriter::new(&mut output, &tracks).unwrap();
        assert!(
            writer
                .write_packet_with_padding(0, 0, 10, true, &[1], padding)
                .is_err()
        );
        assert!(writer.write_packet(0, 0, 10, true, &[1]).is_err());
        assert!(writer.finish().is_err());
    }
}
