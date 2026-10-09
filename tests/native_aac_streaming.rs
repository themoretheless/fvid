use fvid::{container::mp4::Mp4Reader, native_media::decode_mp4_aac_reader};
use std::{
    cell::Cell,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    rc::Rc,
    time::Duration,
};

struct Count<R> {
    inner: R,
    bytes: Rc<Cell<usize>>,
}
impl<R: Read> Read for Count<R> {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        // Exercise legal short reads throughout headers, metadata and packets.
        let length = data.len().min(3);
        let n = self.inner.read(&mut data[..length])?;
        self.bytes.set(self.bytes.get() + n);
        Ok(n)
    }
}
impl<R: Seek> Seek for Count<R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}
struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn large_sparse_mp4_reads_only_index_and_selected_audio_packets() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-streaming-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let _cleanup = Cleanup(dir.clone());
    let fixture = include_bytes!("fixtures/audio/aac-native-edit.m4a");
    let path = dir.join("large.mp4");
    let mut file = File::create(&path).unwrap();
    file.write_all(fixture).unwrap();
    // A real >3 GiB logical container, with an unreferenced sparse media payload.
    // No giant allocation is needed either to produce or to read this fixture.
    let size = 3u32 << 30;
    file.write_all(&size.to_be_bytes()).unwrap();
    file.write_all(b"mdat").unwrap();
    file.set_len(fixture.len() as u64 + u64::from(size))
        .unwrap();
    drop(file);
    let interval = Some((Duration::from_millis(10), Duration::from_millis(75)));
    let mut expected = Vec::new();
    let wanted =
        fvid::native_media::decode_mp4_aac_pcm_interval(fixture, &mut expected, interval).unwrap();
    let bytes = Rc::new(Cell::new(0));
    let reader = Mp4Reader::open(
        Count {
            inner: File::open(&path).unwrap(),
            bytes: bytes.clone(),
        },
        Default::default(),
    )
    .unwrap();
    let mut actual = Vec::new();
    let got = decode_mp4_aac_reader(reader, &mut actual, interval).unwrap();
    assert_eq!(got, wanted);
    assert_eq!(actual, expected);
    assert!(
        bytes.get() < 2 * fixture.len(),
        "read {} bytes",
        bytes.get()
    );
    let destination = dir.join("audio.f32le");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&path)
        .arg(&destination)
        .args(["--from", "0.01", "--to", "0.075", "--quiet"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(status.stdout.is_empty());
    assert_eq!(std::fs::read(destination).unwrap(), expected);
}

#[test]
fn indexed_decode_propagates_writer_failure() {
    struct Fails;
    impl Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("sink failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let source = std::io::Cursor::new(include_bytes!("fixtures/audio/aac-native-edit.m4a"));
    let reader = Mp4Reader::open(source, Default::default()).unwrap();
    assert!(
        decode_mp4_aac_reader(reader, &mut Fails, None)
            .unwrap_err()
            .to_string()
            .contains("sink failed")
    );
}

#[test]
fn large_matroska_skips_sparse_void_and_preserves_trimmed_interval() {
    use fvid::{container::webm::WebmReader, native_media::decode_matroska_aac_reader};
    let dir = std::env::temp_dir().join(format!("fvid-mka-streaming-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let _cleanup = Cleanup(dir.clone());
    let fixture = include_bytes!("fixtures/audio/aac-stereo.mka");
    let mut head = fixture.to_vec();
    let segment = head
        .windows(4)
        .position(|v| v == [0x18, 0x53, 0x80, 0x67])
        .unwrap();
    let width = head[segment + 4].leading_zeros() as usize + 1;
    assert_eq!(width, 8);
    let payload = 3u64 << 30;
    let new_length = head.len() as u64 + 9 + payload;
    let segment_size = new_length - (segment + 4 + width) as u64;
    head[segment + 4..segment + 12].copy_from_slice(&((1u64 << 56) | segment_size).to_be_bytes());
    let path = dir.join("large.mka");
    let mut file = File::create(&path).unwrap();
    file.write_all(&head).unwrap();
    file.write_all(&[0xec]).unwrap(); // EBML Void inside Segment.
    file.write_all(&((1u64 << 56) | payload).to_be_bytes())
        .unwrap();
    file.set_len(new_length).unwrap();
    drop(file);
    let interval = Some((Duration::from_micros(30001), Duration::from_micros(70001)));
    let mut expected = Vec::new();
    let wanted =
        fvid::native_media::decode_matroska_aac_pcm_interval(fixture, &mut expected, interval)
            .unwrap();
    let bytes = Rc::new(Cell::new(0));
    let reader = WebmReader::open(
        Count {
            inner: File::open(&path).unwrap(),
            bytes: bytes.clone(),
        },
        Default::default(),
    )
    .unwrap();
    let mut actual = Vec::new();
    let got = decode_matroska_aac_reader(reader, &mut actual, interval).unwrap();
    assert_eq!(got, wanted);
    assert_eq!(actual, expected);
    assert!(
        bytes.get() < 8 * fixture.len(),
        "read {} bytes",
        bytes.get()
    );
    let output = dir.join("audio.f32le");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&path)
        .arg(&output)
        .args(["--from", "0.030001", "--to", "0.070001", "--quiet"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(status.stdout.is_empty());
    assert_eq!(std::fs::read(output).unwrap(), expected);
}

#[test]
fn sequential_adts_interval_stops_on_an_unbounded_short_read_source() {
    use fvid::{container::adts::StreamReader, native_media::decode_adts_aac_reader};
    struct Repeating {
        data: &'static [u8],
        bytes: Rc<Cell<usize>>,
    }
    impl Read for Repeating {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let length = output.len().min(3);
            let at = self.bytes.get();
            for (index, byte) in output[..length].iter_mut().enumerate() {
                *byte = self.data[(at + index) % self.data.len()];
            }
            self.bytes.set(at + length);
            Ok(length)
        }
    }
    let data = include_bytes!("fixtures/audio/aac-mono-44k.aac");
    let interval = Some((Duration::from_micros(30001), Duration::from_micros(70001)));
    let mut expected = Vec::new();
    let wanted = fvid::native_media::decode_aac_pcm_interval(
        data,
        &mut expected,
        &Default::default(),
        interval,
    )
    .unwrap();
    let bytes = Rc::new(Cell::new(0));
    let reader = StreamReader::open(Repeating {
        data,
        bytes: bytes.clone(),
    })
    .unwrap();
    let mut actual = Vec::new();
    let got = decode_adts_aac_reader(reader, &mut actual, interval).unwrap();
    assert_eq!(got, wanted);
    assert_eq!(actual, expected);
    assert!(bytes.get() < data.len());
}

#[test]
fn sequential_adts_rejects_partial_frames_and_configuration_changes() {
    use fvid::container::adts::{StreamReader, header};
    let data = include_bytes!("fixtures/audio/aac-mono-44k.aac");
    let size = header(data).unwrap().frame_bytes;
    for length in 0..size {
        match StreamReader::open(&data[..length]) {
            Err(_) => assert!(length < 7),
            Ok(mut reader) => assert!(reader.next_packet().is_err(), "accepted {length}/{size}"),
        }
    }
    let mut changed = data[..size].to_vec();
    changed.extend_from_slice(&data[..size]);
    changed[size + 2] ^= 4; // A different sampling frequency in the second header.
    let mut reader = StreamReader::open(changed.as_slice()).unwrap();
    assert!(reader.next_packet().unwrap().is_some());
    assert!(
        reader
            .next_packet()
            .unwrap_err()
            .to_string()
            .contains("configuration")
    );
    let mut reader = StreamReader::open(&data[..size]).unwrap();
    assert_eq!(reader.next_packet().unwrap().unwrap(), data[7..size]);
    assert!(reader.next_packet().unwrap().is_none());
}

#[test]
fn sequential_adts_removes_crc_bytes_and_full_export_does_not_publish_truncation() {
    use fvid::container::adts::{StreamReader, header};
    let data = include_bytes!("fixtures/audio/aac-mono-44k.aac");
    let size = header(data).unwrap().frame_bytes;
    let mut crc = data[..7].to_vec();
    crc[1] &= !1;
    let length = size + 2;
    crc[3] = (crc[3] & !3) | ((length >> 11) as u8 & 3);
    crc[4] = (length >> 3) as u8;
    crc[5] = (crc[5] & 31) | (((length & 7) as u8) << 5);
    let checksum = fvid_media::owned_aac::adts_crc::checksum(crc.as_slice().try_into().unwrap(), &data[7..size], &header(data).unwrap().asc).unwrap();
    crc.extend_from_slice(&checksum.to_be_bytes());
    crc.extend_from_slice(&data[7..size]);
    let mut reader = StreamReader::open(crc.as_slice()).unwrap();
    assert_eq!(reader.next_packet().unwrap().unwrap(), data[7..size]);
    assert!(reader.next_packet().unwrap().is_none());
    let dir = std::env::temp_dir().join(format!("fvid-adts-truncation-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let _cleanup = Cleanup(dir.clone());
    let source = dir.join("broken.aac");
    std::fs::write(&source, &data[..data.len() - 1]).unwrap();
    let destination = dir.join("broken.wav");
    assert!(fvid::native_export::export_aac_pcm(&source, &destination).is_err());
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}
