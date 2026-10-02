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
        let mut library_output = std::io::Cursor::new(Vec::new());
        let library_event = fvid_media::owned_matroska::write_adts(
            fvid_media::owned_aac::adts::StreamReader::open(data.as_slice()).unwrap(),
            &mut library_output, None, None,
        ).unwrap();
        assert_eq!(library_output.get_ref(), output.get_ref());
        assert_eq!(library_event.packets, event.packets);
        assert_eq!(library_event.payload_bytes, event.payload_bytes);
        assert!(!library_event.done);
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

#[test]
fn mp4_aac_edits_and_960_sample_frames_survive_packet_remux() {
    for name in ["aac-native-edit.m4a", "aac-960-48000.m4a"] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        let mut input =
            fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
                .unwrap();
        let index = input
            .tracks()
            .iter()
            .position(|t| t.codec == *b"mp4a")
            .unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        let stats = matroska_write::write_mp4_aac(&mut input, index, &mut out, None, None).unwrap();
        assert!(!stats.done);
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut expected).unwrap();
        let mut actual = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(out.get_ref(), &mut actual, None)
            .unwrap();
        assert_eq!(actual.len(), expected.len(), "{name}");
        assert!(actual == expected, "PCM differs for {name}");
        let mut container =
            webm::WebmReader::open(std::io::Cursor::new(out.get_ref()), Default::default())
                .unwrap();
        container.scan_all().unwrap();
        assert_eq!(container.packets.len() as u64, stats.packets);
        for i in 0..container.packets.len() {
            let mut packet = Vec::new();
            input.read_packet(index, i, &mut packet).unwrap();
            assert_eq!(container.read_packet(i).unwrap(), packet);
        }
    }
}

#[test]
fn mp4_aac_invalid_edits_and_cancellation_never_report_completion() {
    let original = std::fs::read(fixture("aac-native-edit.m4a")).unwrap();
    let elst = original.windows(4).position(|v| v == b"elst").unwrap();
    for media_time in [-1i32, 100_000] {
        let mut bytes = original.clone();
        bytes[elst + 16..elst + 20].copy_from_slice(&media_time.to_be_bytes());
        let mut input =
            fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
                .unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        assert!(matroska_write::write_mp4_aac(&mut input, 0, &mut out, None, None).is_err());
        assert!(out.get_ref().is_empty());
    }
    let mut input =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(original), Default::default())
            .unwrap();
    let mut out = std::io::Cursor::new(Vec::new());
    assert!(matroska_write::write_mp4_aac(&mut input, usize::MAX, &mut out, None, None).is_err());
    assert!(out.get_ref().is_empty());
    let cancel = fvid::media_control::CancelFlag::default();
    let stop = cancel.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets == 1 {
            stop.cancel();
        }
    });
    let error = matroska_write::write_mp4_aac(&mut input, 0, &mut out, Some(&cancel), Some(&hook))
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(!out.get_ref().is_empty(), "cancel after first packet");
    let mut untouched = std::io::Cursor::new(Vec::new());
    assert!(
        matroska_write::write_mp4_aac(&mut input, 0, &mut untouched, Some(&cancel), None).is_err()
    );
    assert!(untouched.get_ref().is_empty());
}

// Use real AAC packet/index bytes and real MP4 tag boxes, adding two chpl
// entries after mdat so sample offsets remain untouched.
fn tagged_aac_mp4() -> Vec<u8> {
    fn children(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            assert!(size >= 8 && at + size <= bytes.len());
            out.push((&bytes[at + 4..at + 8], &bytes[at..at + size]));
            at += size;
        }
        out
    }
    fn atom(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(id);
        out.extend(body);
        out
    }
    let tags =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tags/tags.mp4"))
            .unwrap();
    let moov = children(&tags)
        .into_iter()
        .find(|(id, _)| *id == b"moov")
        .unwrap()
        .1;
    let udta = children(&moov[8..])
        .into_iter()
        .find(|(id, _)| *id == b"udta")
        .unwrap()
        .1;
    let mut metadata = udta[8..].to_vec();
    let mut chapter = vec![1, 0, 0, 0, 0, 0, 0, 0, 2];
    for (start, title) in [(0u64, "Opening"), (500_000, "Глава 2")] {
        chapter.extend(start.to_be_bytes());
        chapter.push(title.len() as u8);
        chapter.extend(title.as_bytes());
    }
    metadata.extend(atom(b"chpl", &chapter));
    let source = std::fs::read(fixture("aac-native-edit.m4a")).unwrap();
    let mut result = Vec::new();
    for (id, bytes) in children(&source) {
        if id != b"moov" {
            result.extend(bytes);
            continue;
        }
        assert!(result.windows(4).any(|w| w == b"mdat"));
        let mut movie = Vec::new();
        for (id, bytes) in children(&bytes[8..]) {
            if id != b"udta" {
                movie.extend(bytes);
            }
        }
        movie.extend(atom(b"udta", &metadata));
        result.extend(atom(b"moov", &movie));
    }
    result
}

#[test]
fn mp4_aac_cli_and_api_publish_metadata_and_preserve_existing_output() {
    let d = dir("mp4-publish");
    let source = d.0.join("source.m4a");
    let bytes = tagged_aac_mp4();
    std::fs::write(&source, &bytes).unwrap();
    let mp4 =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
            .unwrap();
    assert_eq!(mp4.chapters().len(), 2);
    assert_eq!(mp4.chapters()[1].start_ns, 50_000_000);
    assert_eq!(mp4.tags().title, "T");
    let multiple = fixture("two-audio.mp4");
    assert!(!fvid::native_export::is_single_track_mp4_aac(&multiple).unwrap());
    let rejected = d.0.join("rejected.mka");
    assert!(fvid::native_export::remux_mp4_aac_matroska(&multiple, &rejected, None, None).is_err());
    assert!(!rejected.exists());
    let destination = d.0.join("out.mka");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&source)
        .arg(&destination)
        .arg("--progress")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"backend\":\"fvid\""));
    let progress = String::from_utf8_lossy(&run.stderr);
    assert!(progress.lines().last().unwrap().contains("\"done\":true"));
    let result = std::fs::read(&destination).unwrap();
    let mut mkv =
        webm::WebmReader::open(std::io::Cursor::new(&result), Default::default()).unwrap();
    mkv.scan_all().unwrap();
    assert_eq!(&mkv.tags, mp4.tags());
    assert_eq!(
        mkv.chapters,
        fvid::container::matroska_write::FileMetadata::from_mp4(&mp4).chapters
    );
    let mut before = Vec::new();
    let mut after = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut before).unwrap();
    fvid::native_media::decode_matroska_aac_pcm_interval(&result, &mut after, None).unwrap();
    assert!(before == after);
    assert!(
        fvid::native_export::remux_mp4_aac_matroska(&source, &destination, None, None).is_err()
    );
    assert_eq!(std::fs::read(&destination).unwrap(), result);
    let cancel = fvid::media_control::CancelFlag::default();
    let stop = cancel.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets > 0 {
            stop.cancel();
        }
    });
    let cancelled = d.0.join("cancelled.mka");
    assert!(
        fvid::native_export::remux_mp4_aac_matroska(
            &source,
            &cancelled,
            Some(&cancel),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!cancelled.exists());
    assert!(
        !std::fs::read_dir(&d.0).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    #[cfg(feature = "media")]
    {
        let api = d.0.join("api.mkv");
        let stats = fvid::media::remux(&source, &api, &Default::default()).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(std::fs::read(api).unwrap(), result);
    }
}
