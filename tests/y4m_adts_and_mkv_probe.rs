//! Regression tests for Batch 572 (`FVID-0590..0594`):
//! - `FVID-0590`: `fvid_media::owned_probe::probe_adts_with_options` and `try_adts_as`
//!   admit AAC Main (`1`), LC (`2`), SSR (`3`), and LTP (`4`) ADTS headers, report the
//!   profile name (`Main`, `LC`, `SSR`, `LTP`), actual `frame_samples`, and true
//!   `extradata_bytes` (including >2-byte `AudioSpecificConfig` synthesized from an
//!   in-band Program Config Element when `channel_configuration == 0`).
//! - `FVID-0591`: `fvid::playback::Y4mReader::new` and `NativeReader` default an omitted
//!   Y4M `F` frame-rate token to `25:1` (`25 fps`, 40 ms period), matching the Y4M
//!   specification and `fvid_media::owned_y4m::Header::frame_rate`.
//! - `FVID-0592`: `fvid::native_export::inspect_first_y4m_frame_with_filters_and_options`,
//!   `export_y4m_sources_with_options`, and `fvid::native_geometry::VideoGeometry` support
//!   `440` (`[1, 2]`), `411` (`[4, 1]`), and `410` (`[4, 4]`) planar chroma subsampling
//!   layouts across 8-bit `Planar8` and high-bit-depth `Planar` frames (both passthrough
//!   and geometry-transformed).
//! - `FVID-0593`: `fvid_media::owned_y4m::Header::parse` and
//!   `fvid_media::owned_y4m_probe::try_y4m_with_options` accept high-bit-depth `C411p9`..
//!   `C411p16` and `C410p9`..`C410p16` Y4M headers across probe, decode, playback, and
//!   native Y4M export.
//! - `FVID-0594`: `fvid_media::owned_webm_probe::probe_webm_with_options` infers open-ended
//!   Matroska `ChapterAtom` end timestamps (`end_ns == None`) from the next chapter's start
//!   timestamp or the container duration instead of collapsing them to zero-length points.

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::time::Duration;

use fvid::media::CopyOptions;
use fvid::native_export::{concat_y4m_with_options, export_y4m_pipeline_with_options};
use fvid::native_geometry::VideoGeometry;
use fvid::native_pixels::PixelFilters;
use fvid::playback::Y4mReader;
use fvid::playback_native::NativeReader;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-adts-and-mkv-probe")
}

#[test]
fn adts_probe_supports_aac_main_profile_and_inband_pce_extradata_size() {
    let main_path = fixture_dir().join("aac-main-48k-mono.aac");
    for info in [
        fvid_media::owned_probe::probe_adts(&main_path).expect("probe_adts AAC Main"),
        fvid_media::probe(&main_path).expect("fvid_media::probe AAC Main"),
        fvid::media::probe(&main_path).expect("fvid::media::probe AAC Main"),
    ] {
        assert_eq!(info.format, "aac");
        assert_eq!(info.streams.len(), 1);
        let stream = &info.streams[0];
        assert_eq!(stream.codec, "aac");
        assert_eq!(stream.profile.as_deref(), Some("Main"));
        assert_eq!(stream.sample_rate, 48_000);
        assert_eq!(stream.channels, 1);
        assert_eq!(stream.duration, Some(2048));
        assert_eq!(stream.extradata_bytes, 2);
    }

    let pce_path = fixture_dir().join("aac-lc-pce-48k-stereo.aac");
    let pce_info = fvid_media::owned_probe::probe_adts(&pce_path).expect("probe_adts PCE");
    assert_eq!(pce_info.streams.len(), 1);
    let stream = &pce_info.streams[0];
    assert_eq!(stream.codec, "aac");
    assert_eq!(stream.profile.as_deref(), Some("LC"));
    assert_eq!(stream.sample_rate, 48_000);
    assert_eq!(stream.channels, 2);
    assert_eq!(stream.duration, Some(1024));
    assert!(
        stream.extradata_bytes > 2,
        "explicit PCE AudioSpecificConfig must report its full length (> 2 bytes), got {}",
        stream.extradata_bytes
    );
}

#[test]
fn y4m_reader_and_native_reader_default_omitted_frame_rate_to_25_fps() {
    let path = fixture_dir().join("omitted-rate-2x2.y4m");
    let mut reader = Y4mReader::new(BufReader::new(File::open(&path).unwrap()), 1024)
        .expect("open Y4mReader with omitted F rate token");
    assert_eq!(reader.frame_rate(), (25, 1));
    assert_eq!(reader.frame_period(), Duration::from_millis(40));
    assert!(reader.read_frame().expect("read first frame"));
    assert_eq!(
        reader.rgb(),
        &[0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]
    );
    assert!(!reader.read_frame().expect("EOF after first frame"));

    let bytes = std::fs::read(&path).unwrap();
    let mut native =
        NativeReader::without_memory_limit(Cursor::new(bytes)).expect("open NativeReader");
    assert_eq!(native.frame_clock_quantum(), 1);
    assert!(native.read_frame().expect("native read_frame"));
    assert_eq!(native.frame_interval(), Some((0, 1, 25)));
}

#[test]
fn native_y4m_export_and_inspect_support_440_411_and_410_planar_layouts() {
    for (file_name, expected_chroma, expected_bytes) in [
        ("c440-8bit-2x4.y4m", "440", 16u64),
        ("c411-8bit-4x2.y4m", "411", 12u64),
        ("c410-8bit-4x4.y4m", "410", 18u64),
    ] {
        let path = fixture_dir().join(file_name);
        let out_path = std::env::temp_dir().join(format!(
            "fvid-b572-export-{}-{file_name}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_path);
        let event = export_y4m_pipeline_with_options(
            &path,
            &out_path,
            None,
            &VideoGeometry::default(),
            &PixelFilters::default(),
            &CopyOptions::default(),
        )
        .unwrap_or_else(|e| panic!("export passthrough {file_name}: {e}"));
        assert_eq!((event.packets, event.payload_bytes), (1, expected_bytes));
        let exported_text = std::fs::read(&out_path).unwrap();
        let header_line = exported_text.split(|&b| b == b'\n').next().unwrap();
        let header_str = std::str::from_utf8(header_line).unwrap();
        assert!(
            header_str.contains(&format!("C{expected_chroma}")),
            "exported Y4M header {header_str:?} must contain C{expected_chroma}"
        );
        let _ = std::fs::remove_file(&out_path);

        // Verify geometry-transformed export (horizontal_flip) also preserves 440/411/410
        let flipped_geo = VideoGeometry {
            horizontal_flip: true,
            ..Default::default()
        };
        let event_flipped = export_y4m_pipeline_with_options(
            &path,
            &out_path,
            None,
            &flipped_geo,
            &PixelFilters::default(),
            &CopyOptions::default(),
        )
        .unwrap_or_else(|e| panic!("export flipped {file_name}: {e}"));
        assert_eq!(
            (event_flipped.packets, event_flipped.payload_bytes),
            (1, expected_bytes)
        );
        let _ = std::fs::remove_file(&out_path);

        // Verify concat_y4m_with_options (which calls inspect_first_y4m_frame_with_filters_and_options)
        let concat_event = concat_y4m_with_options(
            &[path.clone(), path.clone()],
            &out_path,
            &CopyOptions::default(),
        )
        .unwrap_or_else(|e| panic!("concat {file_name}: {e}"));
        assert_eq!(
            (concat_event.packets, concat_event.payload_bytes),
            (2, expected_bytes * 2)
        );
        let _ = std::fs::remove_file(&out_path);
    }
}

#[test]
fn high_bit_depth_c411p10_and_c410p12_y4m_probe_decode_and_export() {
    for (file_name, expected_w, expected_h, expected_chroma, expected_depth, expected_bytes) in [
        ("c411p10-4x2.y4m", 4usize, 2usize, "411", 10u8, 24u64),
        ("c410p12-4x4.y4m", 4usize, 4usize, "410", 12u8, 36u64),
    ] {
        let path = fixture_dir().join(file_name);
        let probed = fvid_media::owned_y4m_probe::try_y4m(&path)
            .unwrap_or_else(|e| panic!("try_y4m {file_name}: {e}"))
            .unwrap_or_else(|| panic!("try_y4m must recognize {file_name}"));
        assert_eq!(probed.streams[0].width, expected_w as i32);
        assert_eq!(probed.streams[0].height, expected_h as i32);
        assert_eq!(probed.streams[0].duration, Some(1));

        let mut reader = Y4mReader::new(BufReader::new(File::open(&path).unwrap()), 4096)
            .unwrap_or_else(|e| panic!("open Y4mReader {file_name}: {e}"));
        assert_eq!(reader.depth(), expected_depth);
        assert!(reader
            .read_frame()
            .unwrap_or_else(|e| panic!("read_frame {file_name}: {e}")));

        let out_path = std::env::temp_dir().join(format!(
            "fvid-b572-hbd-{}-{file_name}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_path);
        let event = export_y4m_pipeline_with_options(
            &path,
            &out_path,
            None,
            &VideoGeometry::default(),
            &PixelFilters::default(),
            &CopyOptions::default(),
        )
        .unwrap_or_else(|e| panic!("export {file_name}: {e}"));
        assert_eq!((event.packets, event.payload_bytes), (1, expected_bytes));
        let exported_text = std::fs::read(&out_path).unwrap();
        let header_line = exported_text.split(|&b| b == b'\n').next().unwrap();
        let header_str = std::str::from_utf8(header_line).unwrap();
        assert!(
            header_str.contains(&format!("C{expected_chroma}p{expected_depth}")),
            "exported Y4M header {header_str:?} must contain C{expected_chroma}p{expected_depth}"
        );
        let _ = std::fs::remove_file(&out_path);

        let concat_event = concat_y4m_with_options(
            &[path.clone(), path.clone()],
            &out_path,
            &CopyOptions::default(),
        )
        .unwrap_or_else(|e| panic!("concat {file_name}: {e}"));
        assert_eq!(
            (concat_event.packets, concat_event.payload_bytes),
            (2, expected_bytes * 2)
        );
        let _ = std::fs::remove_file(&out_path);
    }
}

#[test]
fn owned_webm_probe_infers_open_ended_chapter_end_from_next_chapter_and_duration() {
    let path = fixture_dir().join("chapters-open-end.mkv");
    let owned = fvid_media::owned_webm_probe::probe_webm(&path).expect("owned probe_webm");
    assert_eq!(owned.chapters.len(), 2);
    assert_eq!(owned.chapters[0].metadata.get("title").map(String::as_str), Some("Intro"));
    assert_eq!(owned.chapters[0].start, 10_000_000);
    assert_eq!(
        owned.chapters[0].end, 40_000_000,
        "first open-ended chapter must end at the next chapter's start_ns"
    );
    assert_eq!(owned.chapters[1].metadata.get("title").map(String::as_str), Some("Outro"));
    assert_eq!(owned.chapters[1].start, 40_000_000);
    assert_eq!(
        owned.chapters[1].end, 100_000_000,
        "last open-ended chapter must end at the container duration_ns"
    );

    let native = fvid::media::probe(&path).expect("fvid::media::probe");
    assert_eq!(native.chapters.len(), 2);
    assert_eq!(
        (native.chapters[0].start, native.chapters[0].end),
        (10_000, 40_000)
    );
    assert_eq!(
        (native.chapters[1].start, native.chapters[1].end),
        (40_000, 100_000)
    );
}
