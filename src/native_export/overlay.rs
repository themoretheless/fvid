//! Owned two-source opaque compositing with presentation-time frame selection.
use crate::{
    Result, invalid,
    media_control::{CancelFlag, ProgressEvent, ProgressHook},
    native_geometry::{GeometryFrame, VideoGeometry},
    playback_native::{NativeReader, RawFrame},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
    sync::atomic::Ordering,
};
struct Picture {
    pts: u64,
    depth: u8,
    planes: GeometryFrame,
}
struct Source {
    reader: NativeReader<BufReader<File>>,
    current: Picture,
    next: Option<Picture>,
    origin: u64,
    colour: crate::color::ColourDescription,
}
fn picture(reader: &mut NativeReader<BufReader<File>>) -> Result<Option<Picture>> {
    let Some(frame) = reader.read_frame_raw()? else {
        return Ok(None);
    };
    let depth = match &frame {
        RawFrame::Avc { picture, .. } => picture.bit_depth,
        RawFrame::Planar(p) => p.depth,
        RawFrame::Planar8(_) | RawFrame::Yuv { .. } => 8,
        _ => return Err(invalid("overlay requires sample planes")),
    };
    let (pts, _, scale) = reader
        .frame_interval()
        .ok_or_else(|| invalid("overlay frame has no timing"))?;
    if scale == 0 {
        return Err(invalid("overlay clock is zero"));
    }
    let pts = u64::try_from(
        pts.checked_mul(1_000_000_000)
            .ok_or_else(|| invalid("overlay clock overflow"))?
            / u128::from(scale),
    )
    .map_err(|_| invalid("overlay timestamp overflow"))?;
    let [w, h] = reader.dimensions();
    let planes = VideoGeometry::default().apply_cropped_display(
        &frame,
        w,
        h,
        reader.rotation(),
        reader.insets(),
    )?;
    Ok(Some(Picture { pts, depth, planes }))
}
impl Source {
    fn open(path: &Path) -> Result<Self> {
        let info = crate::native_probe::probe(path).map_err(|e| invalid(&e))?;
        if info
            .streams
            .iter()
            .filter(|s| s.media_type == "video")
            .count()
            != 1
        {
            return Err(invalid("foreground requires exactly one video track"));
        }
        let mut reader = NativeReader::software(BufReader::new(File::open(path)?), usize::MAX)?;
        let current =
            picture(&mut reader)?.ok_or_else(|| invalid("foreground has no video frames"))?;
        let origin = current.pts;
        let colour = reader.colour();
        let next = picture(&mut reader)?;
        if reader.colour() != colour {
            return Err(invalid("foreground colour encoding changed"));
        }
        Ok(Self {
            reader,
            current,
            next,
            origin,
            colour,
        })
    }
    fn at(&mut self, time: u64, cancel: Option<&CancelFlag>) -> Result<&Picture> {
        while self
            .next
            .as_ref()
            .is_some_and(|p| p.pts.saturating_sub(self.origin) <= time)
        {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                return Err(invalid("media operation cancelled"));
            }
            let next = self.next.take().unwrap();
            if next.pts <= self.current.pts {
                return Err(invalid("foreground timestamps must increase"));
            }
            self.current = next;
            self.next = picture(&mut self.reader)?;
            if self.reader.colour() != self.colour {
                return Err(invalid("foreground colour encoding changed"));
            }
        }
        Ok(&self.current)
    }
}

/// Shared timed compositor for decode and export; source origins stay file-relative.
pub(crate) struct TimedOverlay {
    source: Source,
    x: i64,
    y: i64,
    origin: Option<u64>,
}
impl TimedOverlay {
    pub(crate) fn new(path: &Path, x: i64, y: i64) -> Result<Self> {
        Ok(Self {
            source: Source::open(path)?,
            x,
            y,
            origin: None,
        })
    }
    pub(crate) fn validate_main(
        &mut self,
        reader: &NativeReader<BufReader<File>>,
        pts: u64,
    ) -> Result<()> {
        if reader.colour() != self.source.colour {
            return Err(invalid(
                "overlay inputs require matching colour encoding and range",
            ));
        }
        let aspect = |reader: &NativeReader<BufReader<File>>| {
            let (n, d) = reader.pixel_aspect();
            if matches!(reader.rotation(), 90 | 270) {
                (d, n)
            } else {
                (n, d)
            }
        };
        let (mn, md) = aspect(reader);
        let (fn_, fd) = aspect(&self.source.reader);
        if u64::from(mn) * u64::from(fd) != u64::from(fn_) * u64::from(md)
            || reader.hdr() != self.source.reader.hdr()
        {
            return Err(invalid(
                "overlay inputs require matching display pixel aspect and HDR metadata",
            ));
        }
        self.origin.get_or_insert(pts);
        Ok(())
    }
    pub(crate) fn apply(
        &mut self,
        frame: &mut GeometryFrame,
        depth: u8,
        pts: u64,
        cancel: Option<&CancelFlag>,
    ) -> Result<()> {
        let base = *self.origin.get_or_insert(pts);
        let elapsed = pts
            .checked_sub(base)
            .ok_or_else(|| invalid("overlay main timestamp precedes origin"))?;
        let full_range = self.source.colour.full_range;
        let picture = self.source.at(elapsed, cancel)?;
        crate::native_pixels::overlay_opaque_depth(
            frame,
            &picture.planes,
            depth,
            picture.depth,
            full_range,
            self.x,
            self.y,
        )
    }
}

/// Owned lossless main admission; other main formats retain legacy routing.
pub fn overlay_eligible(main: &Path) -> Result<bool> {
    if crate::native_lossless_y4m::eligible(main)? {
        return Ok(true);
    }
    let mut input = File::open(main)?;
    let mut prefix = [0; 8];
    if input.read(&mut prefix)? != 8 {
        return Ok(false);
    }
    Ok(
        crate::container::mp4::recognizes_prefix(&prefix)
            && crate::native_lossless::eligible(main)?,
    )
}
/// Composite a single foreground video; retain all supported main audio companions.
/// File origins align to the first presented frame, EOF repeats the last frame.
/// Matching colour encoding/range/sampling is required; sample depth is converted.
pub fn overlay_video(
    main: &Path,
    foreground: &Path,
    destination: &Path,
    x: i64,
    y: i64,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<crate::media_info::LosslessStats> {
    overlay_video_transformed(
        main,
        foreground,
        destination,
        x,
        y,
        cancel,
        progress,
        &Default::default(),
        &Default::default(),
    )
}

/// Geometry precedes compositing; supported pixel filters follow compositing.
pub fn overlay_video_transformed(
    main: &Path,
    foreground: &Path,
    destination: &Path,
    x: i64,
    y: i64,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    geometry: &VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
) -> Result<crate::media_info::LosslessStats> {
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err(invalid("owned overlay output requires .mkv"));
    }
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if !overlay_eligible(main)? {
        return Err(invalid(
            "owned overlay requires a supported owned lossless main input",
        ));
    }
    if destination.try_exists()? {
        return Err(invalid("overlay destination already exists"));
    }
    let mut compositor = TimedOverlay::new(foreground, x, y)?;
    let mut main_reader = NativeReader::software(BufReader::new(File::open(main)?), usize::MAX)?;
    main_reader
        .read_frame_raw()?
        .ok_or_else(|| invalid("main input has no frames"))?;
    let (start, _, scale) = main_reader
        .frame_interval()
        .ok_or_else(|| invalid("main frame has no timing"))?;
    if scale == 0 {
        return Err(invalid("overlay clock is zero"));
    }
    let pts = u64::try_from(
        start
            .checked_mul(1_000_000_000)
            .ok_or_else(|| invalid("overlay timestamp overflow"))?
            / u128::from(scale),
    )
    .map_err(|_| invalid("overlay timestamp overflow"))?;
    compositor.validate_main(&main_reader, pts)?;
    drop(main_reader);
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, file) = (0..100)
        .find_map(|_| {
            let path = directory.join(format!(
                ".fvid-overlay-{}-{}.tmp",
                std::process::id(),
                super::NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((super::Temporary(path), file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve overlay output"))??;
    let mut last = None;
    let mut processor = |frame: &mut GeometryFrame, depth: u8, pts: u64| {
        if last.is_some_and(|p| pts <= p) {
            return Err(invalid("main overlay timestamps must increase"));
        }
        last = Some(pts);
        compositor.apply(frame, depth, pts, cancel)
    };
    let mut output = BufWriter::new(file);
    let (stats, event) = if crate::native_lossless_y4m::eligible(main)? {
        crate::native_lossless_y4m::write_processed(
            main,
            &mut output,
            geometry,
            filters,
            cancel,
            progress,
            Some(&mut processor),
        )?
    } else {
        crate::native_lossless::write_mp4_processed(
            main,
            &mut output,
            geometry,
            filters,
            cancel,
            progress,
            Some(&mut processor),
        )?
    };
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    std::fs::hard_link(&temporary.0, destination)?;
    if let Some(hook) = progress {
        hook.emit(ProgressEvent {
            done: true,
            ..event
        });
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreground_selection_uses_its_cadence_and_holds_eof() {
        let path =
            std::env::temp_dir().join(format!("fvid-overlay-selector-{}.y4m", std::process::id()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let mut bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n".to_vec();
        for value in [16, 64, 128] {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[value, value, value, value, 128, 128]);
        }
        std::fs::write(&path, bytes).unwrap();
        let mut source = Source::open(&path).unwrap();
        let mut compositor = TimedOverlay::new(&path, 0, 0).unwrap();
        // An interval beginning one second into the file must not restart foreground.
        compositor.origin = Some(500_000_000);
        let mut target = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0; 6],
        };
        compositor
            .apply(&mut target, 8, 1_500_000_000, None)
            .unwrap();
        assert_eq!(target.data, [128, 128, 128, 128, 128, 128]);

        for (time, value) in [
            (0, 16),
            (499_999_999, 16),
            (500_000_000, 64),
            (999_999_999, 64),
            (1_000_000_000, 128),
            (5_000_000_000, 128),
        ] {
            assert_eq!(source.at(time, None).unwrap().planes.data[0], value);
        }
    }
}
