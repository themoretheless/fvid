use fvid_media::owned_aac::decode_adts_pcm;
use std::{
    cell::Cell,
    io::{Cursor, Read, Write},
    rc::Rc,
    time::Duration,
};
struct Short<R> {
    source: R,
    count: Rc<Cell<usize>>,
}
impl<R: Read> Read for Short<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let size = out.len().min(3);
        let got = self.source.read(&mut out[..size])?;
        self.count.set(self.count.get() + got);
        Ok(got)
    }
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}
#[test]
fn library_stream_matches_frontend_pcm_and_fractional_sample_clock() {
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-pce-wide8.aac",
        "aac-tns.aac",
    ] {
        let bytes = fixture(&format!("audio/{name}"));
        for interval in [
            None,
            Some((
                Duration::from_nanos(12345678),
                Duration::from_nanos(59876543),
            )),
        ] {
            let mut expected = Vec::new();
            let reader = fvid::container::adts::StreamReader::open(Cursor::new(&bytes)).unwrap();
            let reference =
                fvid::native_media::decode_adts_aac_reader(reader, &mut expected, interval)
                    .unwrap();
            let count = Rc::new(Cell::new(0));
            let source = Short {
                source: Cursor::new(&bytes),
                count: count.clone(),
            };
            let mut actual = Vec::new();
            let result =
                decode_adts_pcm(source, &mut actual, interval, &Default::default()).unwrap();
            assert_eq!(actual, expected, "{name}");
            assert_eq!(
                (
                    result.sample_frames,
                    result.decoded_frames,
                    result.sample_rate,
                    result.channels
                ),
                (
                    reference.sample_frames,
                    reference.decoded_frames,
                    reference.sample_rate,
                    reference.channels
                )
            );
            if interval.is_none() {
                assert_eq!(count.get(), bytes.len());
            }
        }
    }
}
#[test]
fn count_and_range_stop_before_truncated_header_with_exact_audible_prefix() {
    let bytes = fixture("playback-errors/aac-packet-prefix.aac");
    let video = fvid::native_probe::probe(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/aac-packet-prefix.y4m"),
    )
    .unwrap();
    assert_eq!(video.streams[0].duration, Some(3));
    let mut output = Vec::new();
    assert!(
        decode_adts_pcm(Cursor::new(&bytes), &mut output, None, &Default::default())
            .unwrap_err()
            .to_string()
            .contains("fill whole buffer")
    );
    let mut reference = Vec::new();
    let mut reader = fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&bytes)).unwrap();
    let mut decoder =
        fvid_media::owned_aac::NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
    for _ in 0..3 {
        for sample in decoder
            .decode(&reader.next_packet().unwrap().unwrap())
            .unwrap()
        {
            reference.extend_from_slice(&sample.to_le_bytes());
        }
    }
    for interval in [
        None,
        Some((
            Duration::from_nanos(12345678),
            Duration::from_nanos(69659863),
        )),
    ] {
        let count = Rc::new(Cell::new(0));
        let source = Short {
            source: Cursor::new(&bytes),
            count: count.clone(),
        };
        let options = fvid_control::CopyOptions {
            max_packets: Some(3),
            max_packet_bytes: 1024,
            ..Default::default()
        };
        let mut output = Vec::new();
        let stats = decode_adts_pcm(source, &mut output, interval, &options).unwrap();
        assert_eq!(stats.decoded_frames, 3);
        assert_eq!(count.get(), bytes.len() - 6);
        let from = interval.map_or(0, |(from, _)| {
            (from.as_nanos() * 44100).div_ceil(1_000_000_000) as usize
        });
        assert_eq!(output, &reference[from * 4..]);
        assert_eq!(stats.sample_frames, 3072 - from as u64);
    }
}
#[test]
fn cancellation_and_zero_limit_happen_before_source_read() {
    struct NoRead;
    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("source read")
        }
    }
    let cancel = fvid_control::CancelFlag::default();
    cancel.cancel();
    for options in [
        fvid_control::CopyOptions {
            cancel: Some(cancel),
            ..Default::default()
        },
        fvid_control::CopyOptions {
            max_packets: Some(0),
            ..Default::default()
        },
    ] {
        let mut output = Vec::new();
        assert!(decode_adts_pcm(NoRead, &mut output, None, &options).is_err());
        assert!(output.is_empty());
    }
}
#[test]
fn writer_errors_and_mid_decode_cancellation_never_report_completion() {
    struct Fails;
    impl Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("sink failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let bytes = fixture("audio/aac-mono-44k.aac");
    assert!(
        decode_adts_pcm(Cursor::new(&bytes), &mut Fails, None, &Default::default())
            .unwrap_err()
            .to_string()
            .contains("sink failed")
    );
    let cancel = fvid_control::CancelFlag::default();
    let hook_cancel = cancel.clone();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets == 1 {
                hook_cancel.cancel();
            }
        })),
        ..Default::default()
    };
    let mut output = Vec::new();
    assert!(
        decode_adts_pcm(Cursor::new(bytes), &mut output, None, &options)
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert_eq!(output.len(), 1024 * 4);
}

#[test]
fn pce_payload_limit_is_enforced_before_bootstrap_read() {
    let bytes = fixture("audio/aac-pce-wide8.aac");
    let header = fvid_media::owned_aac::adts::header(&bytes).unwrap();
    assert_eq!(header.channels, 0);
    struct HeaderOnly(Cursor<Vec<u8>>);
    impl Read for HeaderOnly {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.0.position() < 7, "PCE payload read before refusal");
            self.0.read(out)
        }
    }
    let options = fvid_control::CopyOptions {
        max_packet_bytes: header.frame_bytes - header.header_bytes - 1,
        ..Default::default()
    };
    let mut output = Vec::new();
    let error = decode_adts_pcm(
        HeaderOnly(Cursor::new(bytes[..7].to_vec())),
        &mut output,
        None,
        &options,
    )
    .unwrap_err();
    assert!(error.to_string().contains("ADTS packet exceeds budget"));
    assert!(output.is_empty());
}
