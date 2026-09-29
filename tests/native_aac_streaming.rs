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
