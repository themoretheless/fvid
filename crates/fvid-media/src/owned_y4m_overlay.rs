//! Streaming Y4M overlay scheduling on the primary presentation clock.
use crate::{
    owned_frame::GeometryFrame,
    owned_y4m::{Header, PixelFormat, line},
};
use fvid_media_info::OverlaySpec;
use std::{
    fs::File,
    io::{BufReader, Read},
};
type Result<T> = std::result::Result<T, String>;

pub(crate) struct OverlayReader {
    input: BufReader<File>,
    header: Header,
    frame: GeometryFrame,
    next: u64,
    eof: bool,
    main_rate: [i32; 2],
    x: i32,
    y: i32,
}
impl OverlayReader {
    pub(crate) fn open(main: &Header, spec: &OverlaySpec) -> Result<Self> {
        let mut input = BufReader::new(File::open(&spec.path).map_err(|e| e.to_string())?);
        let mut bytes = Vec::new();
        if !line(&mut input, &mut bytes)? {
            return Err("empty Y4M overlay source".into());
        }
        let header = Header::parse(&bytes)?;
        // Legacy overlay defaults to 8-bit 4:2:0; other output colour conversions
        // remain outside this route until implemented by the owned pipeline.
        for h in [main, &header] {
            if h.format != PixelFormat::Yuv420
                || h.depth() != 8
                || h.tokens
                    .iter()
                    .any(|t| matches!(t.as_str(), "C420mpeg2" | "C420paldv"))
            {
                return Err("owned scheduled overlay requires 8-bit JPEG-sited YUV420".into());
            }
        }
        if main.full_range()? != header.full_range()? {
            return Err("overlay requires matching colour range".into());
        }
        if spec.x % 2 != 0 || spec.y % 2 != 0 {
            return Err("overlay placement must align with chroma samples".into());
        }
        header.frame_rate()?;
        Ok(Self {
            frame: GeometryFrame {
                width: header.width,
                height: header.height,
                subsampling: Some([2, 2]),
                data: Vec::new(),
            },
            main_rate: main.frame_rate()?,
            input,
            header,
            next: 0,
            eof: false,
            x: spec.x,
            y: spec.y,
        })
    }
    pub(crate) fn apply(&mut self, output: &Header, frame: &mut Vec<u8>, index: u64) -> Result<()> {
        let [n, d] = self.header.frame_rate()?;
        let target = u128::from(index) * self.main_rate[1] as u128 * n as u128
            / (self.main_rate[0] as u128 * d as u128);
        let target = u64::try_from(target).map_err(|_| "overlay frame clock overflow")?;
        let mut marker = Vec::new();
        while !self.eof && self.next <= target {
            if !line(&mut self.input, &mut marker)? {
                self.eof = true;
                if self.next == 0 {
                    return Err("Y4M overlay source has no frames".into());
                }
                break;
            }
            if marker != b"FRAME\n" && !marker.starts_with(b"FRAME ") {
                return Err("expected overlay Y4M FRAME marker".into());
            }
            let size = self.header.frame_len()?;
            if self.frame.data.is_empty() {
                self.frame
                    .data
                    .try_reserve_exact(size)
                    .map_err(|e| e.to_string())?;
                self.frame.data.resize(size, 0);
            }
            self.input
                .read_exact(&mut self.frame.data)
                .map_err(|e| format!("overlay frame payload: {e}"))?;
            self.next = self
                .next
                .checked_add(1)
                .ok_or("overlay frame count overflow")?;
        }
        let (sx, sy) = output.format.subsampling();
        let mut destination = GeometryFrame {
            width: output.width,
            height: output.height,
            subsampling: Some([sx, sy]),
            data: std::mem::take(frame),
        };
        let result = crate::owned_overlay::overlay_opaque(
            &mut destination,
            &self.frame,
            output.depth(),
            i64::from(self.x),
            i64::from(self.y),
        );
        *frame = destination.data;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fvid_media_info::{DecodeTransform, LosslessTransform};
    use std::{io::Cursor, path::Path};
    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
    }
    fn transform() -> DecodeTransform {
        DecodeTransform {
            overlay: Some(OverlaySpec {
                path: fixtures().join("overlay-secondary-clock.y4m"),
                x: 2,
                y: 2,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn synchronizes_rates_repeats_last_frame_and_preserves_interval_clock() {
        let primary =
            include_bytes!("../../../tests/fixtures/playback-errors/overlay-primary-clock.y4m");
        for interval in [None, Some((500_000, 1_250_000))] {
            let mut transform = transform();
            transform.interval = interval;
            let mut indices = Vec::new();
            let stats = crate::owned_y4m_decode::visit_reader_transformed(
                Cursor::new(primary),
                &transform,
                |header, pixels, pts, duration| {
                    assert_eq!((header.width, header.height), (4, 4));
                    assert_eq!(duration, 250_000_000);
                    let index = (pts / duration) as u8;
                    indices.push(index);
                    let overlay = if index < 2 {
                        (50, 80, 160)
                    } else {
                        (100, 90, 170)
                    };
                    let mut expected = vec![10 + index; 16];
                    for at in [10, 11, 14, 15] {
                        expected[at] = overlay.0;
                    }
                    expected
                        .extend_from_slice(&[128, 128, 128, overlay.1, 128, 128, 128, overlay.2]);
                    assert_eq!(pixels, expected);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(
                indices,
                if interval.is_some() {
                    vec![2, 3, 4]
                } else {
                    vec![0, 1, 2, 3, 4, 5]
                }
            );
            assert_eq!(stats.video_frames, indices.len() as u64);
        }
    }
    #[test]
    fn exports_overlay_through_owned_public_api() {
        let source = fixtures().join("overlay-primary-clock.y4m");
        let transform = LosslessTransform {
            overlay: transform().overlay,
            ..Default::default()
        };
        let options = crate::CopyOptions::default();
        assert!(crate::owned_lossless::supports(
            &source, &transform, &options
        ));
        let output =
            std::env::temp_dir().join(format!("fvid-scheduled-overlay-{}.mkv", std::process::id()));
        let spec = transform.overlay.as_ref().unwrap();
        let plan = crate::plan_overlay(&source, &spec.path, spec.x, spec.y, &options).unwrap();
        assert_eq!(plan.command, "overlay");
        assert_eq!(plan.inputs, [source.clone(), spec.path.clone()]);
        assert!(plan.notes[0].starts_with("backend: owned"));
        let stats =
            crate::overlay_video(&source, &spec.path, &output, spec.x, spec.y, &options).unwrap();
        assert_eq!(stats.video_frames, 6);
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.encoder, "ffv1");
        let info = crate::owned_webm_probe::probe_webm(&output).unwrap();
        assert_eq!(info.streams[0].codec, "ffv1");
        std::fs::remove_file(output).unwrap();
    }
    #[test]
    fn unsupported_colour_conversion_keeps_legacy_route() {
        let source = fixtures().join("overlay-primary-clock.y4m");
        let mut transform = transform();
        transform.overlay.as_mut().unwrap().x = 1;
        assert!(!crate::owned_y4m_decode::supports_transformed(
            &source, &transform
        ));
        transform.overlay.as_mut().unwrap().x = 2;
        transform.overlay.as_mut().unwrap().path = fixtures().join("shuffleplanes-444-8.y4m");
        assert!(!crate::owned_y4m_decode::supports_transformed(
            &source, &transform
        ));
    }
}
