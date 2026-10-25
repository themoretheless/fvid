use fvid::container::mp4_relocate::fast_start_with_memory_limit;
use std::io::{Cursor, Read, Seek, SeekFrom};
struct Count {
    inner: Cursor<&'static [u8]>,
    bytes: usize,
}
impl Read for Count {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes += n;
        Ok(n)
    }
}
impl Seek for Count {
    fn seek(&mut self, at: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(at)
    }
}
const VIDEO: &[u8] =
    include_bytes!("fixtures/playback-errors/mp4-relocation-admission/many-boxes.mp4");
#[test]
fn metadata_budget_refuses_before_scanning_or_publishing() {
    let mut reader = Count {
        inner: Cursor::new(VIDEO),
        bytes: 0,
    };
    let mut output = Vec::new();
    let error = fast_start_with_memory_limit(&mut reader, &mut output, None, None, Some(65536))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("controlled memory budget exceeded"),
        "{error}"
    );
    assert_eq!(reader.bytes, 0);
    assert!(output.is_empty());
}
#[test]
fn sufficient_budget_preserves_all_fragment_bytes() {
    let mut output = Vec::new();
    fast_start_with_memory_limit(
        &mut Cursor::new(VIDEO),
        &mut output,
        None,
        None,
        Some(2 << 20),
    )
    .unwrap();
    assert_eq!(output, VIDEO);
}
#[cfg(feature = "player")]
#[test]
fn padded_video_plays_exact_frames_and_rewinds() {
    use fvid::playback_native::NativeReader;
    let mut reader =
        NativeReader::software(std::io::BufReader::new(Cursor::new(VIDEO)), 32 << 20).unwrap();
    for pass in 0..2 {
        for i in 0..3 {
            let frame = reader.read_frame_raw().unwrap().unwrap();
            assert_eq!(frame.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            assert_eq!(reader.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        assert!(reader.read_frame_raw().unwrap().is_none());
        if pass == 0 {
            reader.rewind().unwrap();
        }
    }
}
