//! Platform-independent virtual-camera timing. Host times are monotonic nanoseconds.
//! System device publication and frame transport are implemented separately.
use crate::{Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraTick {
    pub sequence: u64,
    /// Camera presentation time never follows file seeks or loops backwards.
    pub host_time_ns: u64,
    /// Position requested from the file decoder.
    pub media_time_ns: u64,
}
pub struct CameraClock {
    rate_num: u32,
    rate_den: u32,
    start: u64,
    last_poll: u64,
    last_sequence: Option<u64>,
    media_anchor: u64,
    host_anchor: u64,
    paused: bool,
    duration: Option<u64>,
}
impl CameraClock {
    pub fn new(rate_num: u32, rate_den: u32, host_time_ns: u64) -> Result<Self> {
        if rate_num == 0 || rate_den == 0 || u64::from(rate_num) > 240 * u64::from(rate_den) {
            return Err(invalid("invalid virtual-camera frame rate"));
        }
        Ok(Self {
            rate_num,
            rate_den,
            start: host_time_ns,
            last_poll: host_time_ns,
            last_sequence: None,
            media_anchor: 0,
            host_anchor: host_time_ns,
            paused: false,
            duration: None,
        })
    }
    fn validate_time(&self, now: u64) -> Result<()> {
        if now < self.last_poll {
            return Err(invalid("virtual-camera host clock moved backwards"));
        }
        Ok(())
    }
    fn media_at(&self, now: u64) -> Result<u64> {
        let position = if self.paused {
            self.media_anchor
        } else {
            self.media_anchor
                .checked_add(now - self.host_anchor)
                .ok_or_else(|| invalid("virtual-camera media time overflow"))?
        };
        Ok(self
            .duration
            .map_or(position, |duration| position % duration))
    }
    /// Change file position without changing the camera's presentation timeline.
    pub fn seek(&mut self, media_time_ns: u64, now: u64) -> Result<()> {
        self.validate_time(now)?;
        self.media_anchor = self.duration.map_or(media_time_ns, |d| media_time_ns % d);
        self.host_anchor = now;
        self.last_poll = now;
        Ok(())
    }
    pub fn set_paused(&mut self, paused: bool, now: u64) -> Result<()> {
        self.validate_time(now)?;
        let position = self.media_at(now)?;
        self.media_anchor = position;
        self.host_anchor = now;
        self.last_poll = now;
        self.paused = paused;
        Ok(())
    }
    /// A positive duration enables looping. None leaves EOF handling to the decoder.
    pub fn set_loop_duration(&mut self, duration: Option<u64>, now: u64) -> Result<()> {
        self.validate_time(now)?;
        if duration == Some(0) {
            return Err(invalid("virtual-camera loop duration is zero"));
        }
        let position = self.media_at(now)?;
        self.media_anchor = duration.map_or(position, |d| position % d);
        self.duration = duration;
        self.host_anchor = now;
        self.last_poll = now;
        Ok(())
    }
    /// At most one tick is returned per call. A late consumer skips missed slots,
    /// rather than accumulating a queue. Paused media still produces camera ticks.
    pub fn poll(&mut self, now: u64) -> Result<Option<CameraTick>> {
        self.validate_time(now)?;
        let period = u128::from(self.rate_den) * 1_000_000_000;
        let sequence = (u128::from(now - self.start) * u128::from(self.rate_num) / period) as u64;
        if self.last_sequence == Some(sequence) {
            self.last_poll = now;
            return Ok(None);
        }
        let media_time_ns = self.media_at(now)?;
        // Timestamp actual delivery time; skipped slots do not produce stale frames.
        let tick = CameraTick {
            sequence,
            host_time_ns: now,
            media_time_ns,
        };
        self.last_poll = now;
        self.last_sequence = Some(sequence);
        Ok(Some(tick))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rational_rate_does_not_accumulate_rounding_error_or_backlog() {
        let mut clock = CameraClock::new(30000, 1001, 0).unwrap();
        assert_eq!(clock.poll(0).unwrap().unwrap().sequence, 0);
        assert!(clock.poll(33_366_666).unwrap().is_none());
        assert_eq!(clock.poll(33_366_667).unwrap().unwrap().sequence, 1);
        let tick = clock.poll(1_001_000_000_000).unwrap().unwrap();
        assert_eq!(tick.sequence, 30000);
        assert!(clock.poll(tick.host_time_ns).unwrap().is_none());
        assert!(clock.poll(0).is_err());
    }
    #[test]
    fn seek_pause_and_loop_preserve_camera_time() {
        let mut clock = CameraClock::new(25, 1, 100).unwrap();
        clock.set_loop_duration(Some(100_000_000), 100).unwrap();
        let first = clock.poll(40_000_100).unwrap().unwrap();
        clock.set_paused(true, 50_000_100).unwrap();
        let paused = clock.poll(80_000_100).unwrap().unwrap();
        assert_eq!(paused.media_time_ns, 50_000_000);
        clock.seek(10_000_000, 90_000_100).unwrap();
        assert_eq!(
            clock.poll(120_000_100).unwrap().unwrap().media_time_ns,
            10_000_000
        );
        clock.set_paused(false, 120_000_100).unwrap();
        let looped = clock.poll(240_000_100).unwrap().unwrap();
        assert_eq!(looped.media_time_ns, 30_000_000);
        assert!(looped.host_time_ns > first.host_time_ns);
        assert!(clock.set_loop_duration(Some(0), 240_000_100).is_err());
        assert_eq!(
            clock.poll(280_000_100).unwrap().unwrap().media_time_ns,
            70_000_000
        );
    }
}

/// Visible dimensions after container display-oriented crop borders.
pub fn visible_dimensions(source: [usize; 2], insets: [u32; 4]) -> Result<[usize; 2]> {
    let [left, top, right, bottom] = insets.map(|n| n as usize);
    let width = source[0].checked_sub(left).and_then(|n| n.checked_sub(right))
        .filter(|&n| n > 0).ok_or_else(|| invalid("camera crop removes picture width"))?;
    let height = source[1].checked_sub(top).and_then(|n| n.checked_sub(bottom))
        .filter(|&n| n > 0).ok_or_else(|| invalid("camera crop removes picture height"))?;
    Ok([width, height])
}

/// One fixed-size BGRA frame shared by a producer and a camera transport.
/// Wrap in Arc to share. Storage is allocated once and never grows; snapshots
/// copy into caller-owned memory so consumers cannot retain internal buffers.
pub struct LatestFrame {
    width: usize,
    height: usize,
    state: std::sync::Mutex<FrameState>,
}
struct FrameState {
    bgra: Vec<u8>,
    tick: Option<CameraTick>,
    closed: bool,
}
impl LatestFrame {
    pub fn new(width: usize, height: usize, memory_limit: usize) -> Result<Self> {
        let bytes = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n > 0 && n <= memory_limit)
            .ok_or_else(|| invalid("virtual-camera frame exceeds memory budget"))?;
        let mut bgra = Vec::new();
        bgra.try_reserve_exact(bytes)
            .map_err(|_| invalid("cannot allocate virtual-camera frame"))?;
        bgra.resize(bytes, 0);
        Ok(Self {
            width,
            height,
            state: std::sync::Mutex::new(FrameState {
                bgra,
                tick: None,
                closed: false,
            }),
        })
    }
    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }
    /// Accept tightly packed RGB from the FVid decoder. Camera format stays BGRA.
    /// A newer publication overwrites the previous one even if it was never read.
    pub fn publish_rgb(&self, tick: CameraTick, rgb: &[u8]) -> Result<()> {
        self.publish_rgb_cropped(tick, rgb, self.dimensions(), [0; 4])
    }
    /// Crop display-oriented RGB directly into the fixed BGRA publication buffer.
    pub fn publish_rgb_cropped(&self, tick: CameraTick, rgb: &[u8],
        source: [usize; 2], insets: [u32; 4]) -> Result<()> {
        if visible_dimensions(source, insets)? != self.dimensions() {
            return Err(invalid("camera visible source and output dimensions differ"));
        }
        if source[0].checked_mul(source[1]).and_then(|n| n.checked_mul(3)) != Some(rgb.len()) {
            return Err(invalid("virtual-camera RGB frame size mismatch"));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("virtual-camera frame lock poisoned"))?;
        if state.closed {
            return Err(invalid("virtual-camera producer is closed"));
        }
        if state.tick.is_some_and(|last| {
            tick.sequence <= last.sequence || tick.host_time_ns <= last.host_time_ns
        }) {
            return Err(invalid("virtual-camera frame timestamp is not increasing"));
        }
        let [left, top, _, _] = insets.map(|n| n as usize);
        for y in 0..self.height {
            let offset = ((y + top) * source[0] + left) * 3;
            let row = &rgb[offset..offset + self.width * 3];
            let out = &mut state.bgra[y * self.width * 4..(y + 1) * self.width * 4];
            for (pixel, bgra) in row.as_chunks::<3>().0.iter().zip(out.as_chunks_mut::<4>().0.iter_mut()) {
                bgra.copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            }
        }
        state.tick = Some(tick);
        Ok(())
    }
    /// Fit cropped RGB into the fixed camera buffer without allocating another frame.
    /// Aspect ratios describe source and destination samples, so the camera's
    /// negotiated format remains stable when the file changes size or aspect.
    pub fn publish_rgb_fitted_aspect(
        &self, tick: CameraTick, rgb: &[u8], source: [usize; 2],
        insets: [u32; 4], source_aspect: [u32; 2], target_aspect: [u32; 2],
    ) -> Result<()> {
        let visible = visible_dimensions(source, insets)?;
        if source_aspect.contains(&0) || target_aspect.contains(&0)
            || source.into_iter().chain(self.dimensions()).any(|n| n == 0 || n > 4096)
            || source[0].checked_mul(source[1]).and_then(|n| n.checked_mul(3)) != Some(rgb.len())
        { return Err(invalid("invalid camera RGB scaling geometry")); }
        if visible == self.dimensions() && source_aspect == target_aspect {
            return self.publish_rgb_cropped(tick, rgb, source, insets);
        }
        let display_w = visible[0] as u128 * u128::from(source_aspect[0]) * u128::from(target_aspect[1]);
        let display_h = visible[1] as u128 * u128::from(source_aspect[1]) * u128::from(target_aspect[0]);
        let (w, h) = if display_w * self.height as u128 > display_h * self.width as u128 {
            (self.width, (display_h * self.width as u128 / display_w).max(1) as usize)
        } else {
            ((display_w * self.height as u128 / display_h).max(1) as usize, self.height)
        };
        let mut state = self.state.lock().map_err(|_| invalid("virtual-camera frame lock poisoned"))?;
        if state.closed { return Err(invalid("virtual-camera producer is closed")); }
        if state.tick.is_some_and(|last| tick.sequence <= last.sequence || tick.host_time_ns <= last.host_time_ns) {
            return Err(invalid("virtual-camera frame timestamp is not increasing"));
        }
        for pixel in state.bgra.as_chunks_mut::<4>().0.iter_mut() { pixel.copy_from_slice(&[0,0,0,255]); }
        let (left, top) = ((self.width - w) / 2, (self.height - h) / 2);
        for y in 0..h {
            for x in 0..w {
                let src = (((y * visible[1] / h) + insets[1] as usize) * source[0]
                    + x * visible[0] / w + insets[0] as usize) * 3;
                let dst = ((top + y) * self.width + left + x) * 4;
                state.bgra[dst..dst+4].copy_from_slice(&[rgb[src+2],rgb[src+1],rgb[src],255]);
            }
        }
        state.tick = Some(tick);
        Ok(())
    }
    /// Copy the latest frame if newer than `after_sequence`. None requests an
    /// initial snapshot. No frame (or no change) leaves the destination untouched.
    pub fn copy_latest(
        &self,
        after_sequence: Option<u64>,
        bgra: &mut [u8],
    ) -> Result<Option<CameraTick>> {
        let state = self
            .state
            .lock()
            .map_err(|_| invalid("virtual-camera frame lock poisoned"))?;
        if bgra.len() != state.bgra.len() {
            return Err(invalid("virtual-camera snapshot size mismatch"));
        }
        let Some(tick) = state.tick else {
            return Ok(None);
        };
        if after_sequence.is_some_and(|s| s >= tick.sequence) {
            return Ok(None);
        }
        bgra.copy_from_slice(&state.bgra);
        Ok(Some(tick))
    }
    /// Stop new publications, preserving the last frame for hold-last-frame output.
    pub fn close(&self) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| invalid("virtual-camera frame lock poisoned"))?
            .closed = true;
        Ok(())
    }
    pub fn is_closed(&self) -> Result<bool> {
        Ok(self
            .state
            .lock()
            .map_err(|_| invalid("virtual-camera frame lock poisoned"))?
            .closed)
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    #[test]
    fn latest_frame_replaces_stale_frames_and_holds_on_disconnect() {
        let frame = LatestFrame::new(2, 1, 8).unwrap();
        let mut out = [99; 8];
        assert!(frame.copy_latest(None, &mut out).unwrap().is_none());
        assert_eq!(out, [99; 8]);
        for sequence in 0..1000 {
            frame
                .publish_rgb(
                    CameraTick {
                        sequence,
                        host_time_ns: sequence + 1,
                        media_time_ns: 0,
                    },
                    &[1, 2, 3, 4, 5, sequence as u8],
                )
                .unwrap();
        }
        let tick = frame.copy_latest(None, &mut out).unwrap().unwrap();
        assert_eq!(tick.sequence, 999);
        assert_eq!(out, [3, 2, 1, 255, 231, 5, 4, 255]);
        assert!(frame.copy_latest(Some(999), &mut out).unwrap().is_none());
        assert!(frame.publish_rgb(tick, &[0; 6]).is_err());
        frame.close().unwrap();
        assert!(frame.is_closed().unwrap());
        assert!(
            frame
                .publish_rgb(
                    CameraTick {
                        sequence: 1000,
                        host_time_ns: 1001,
                        media_time_ns: 0
                    },
                    &[0; 6]
                )
                .is_err()
        );
        assert_eq!(frame.copy_latest(None, &mut out).unwrap(), Some(tick));
        assert!(LatestFrame::new(usize::MAX, 2, usize::MAX).is_err());
        assert!(LatestFrame::new(2, 1, 7).is_err());
    }
}

/// File-to-frame-buffer bridge for FVid's native Y4M reader. EOF holds the last
/// frame. Seeking backwards replays from the start; no frame index is retained.
pub struct Y4mCameraSource<R> {
    source: NativeCameraSource<R>,
}
impl<R: std::io::BufRead + std::io::Seek> Y4mCameraSource<R> {
    pub fn new(reader: crate::playback::Y4mReader<R>) -> Self {
        Self {
            source: NativeCameraSource::new(crate::playback_native::NativeReader::Y4m(reader)),
        }
    }
    pub fn publish(&mut self, tick: CameraTick, destination: &LatestFrame) -> Result<bool> {
        self.source.publish(tick, destination)
    }
}

/// End-of-file policy for a native camera source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraEndBehavior {
    #[default]
    Hold,
    Loop,
}

/// File source for owned native video formats, driven by camera media time.
/// Fits the visible display area after crop and rotation into the fixed camera
/// format, preserving display aspect across source resolution changes.
/// Backward seeks replay from the beginning; EOF holds the final frame by default; an explicit loop policy repeats the file.
/// This produces BGRA frames, not an installed OS camera device.
pub struct NativeCameraSource<R> {
    reader: crate::playback_native::NativeReader<R>,
    eof: bool,
    target_aspect: [u32; 2],
    end_behavior: CameraEndBehavior,
    duration_ns: Option<u64>,
}
impl<R: std::io::BufRead + std::io::Seek> NativeCameraSource<R> {
    pub fn new(reader: crate::playback_native::NativeReader<R>) -> Self {
        let (num, den) = reader.pixel_aspect();
        Self { reader, eof: false, target_aspect: [num, den], end_behavior: CameraEndBehavior::Hold, duration_ns: None }
    }
    /// Choose whether end of file holds the last frame or repeats the file.
    pub fn with_end_behavior(mut self, behavior: CameraEndBehavior) -> Self {
        self.set_end_behavior(behavior);
        self
    }
    pub fn set_end_behavior(&mut self, behavior: CameraEndBehavior) {
        self.end_behavior = behavior;
    }
    fn select(&mut self, media_time_ns: u64) -> Result<()> {
        if let Some((start, _, scale)) = self.reader.frame_interval()
            && u128::from(media_time_ns) * u128::from(scale) < start * 1_000_000_000 {
                self.reader.rewind()?;
                self.eof = false;
            }
        while !self.eof {
            if let Some((_, end, scale)) = self.reader.frame_interval()
                && u128::from(media_time_ns) * u128::from(scale) < end * 1_000_000_000 {
                    break;
                }
            if !self.reader.read_frame()? {
                self.eof = true;
            }
        }
        Ok(())
    }
    pub fn publish(&mut self, tick: CameraTick, destination: &LatestFrame) -> Result<bool> {
        let looping = self.end_behavior == CameraEndBehavior::Loop;
        let time = if looping {
            self.duration_ns.map_or(tick.media_time_ns, |duration| tick.media_time_ns % duration)
        } else { tick.media_time_ns };
        self.select(time)?;
        if self.eof && looping && self.duration_ns.is_none()
            && let Some((_, end, scale)) = self.reader.frame_interval() {
                let duration = (end * 1_000_000_000).div_ceil(u128::from(scale));
                let duration = u64::try_from(duration).map_err(|_| invalid("camera source duration exceeds nanosecond clock"))?;
                if duration == 0 { return Err(invalid("camera loop requires a positive duration")); }
                self.duration_ns = Some(duration);
                self.reader.rewind()?;
                self.eof = false;
                self.select(tick.media_time_ns % duration)?;
            }
        if self.reader.frame_interval().is_none() {
            return Ok(false);
        }
        let (num, den) = self.reader.pixel_aspect();
        destination.publish_rgb_fitted_aspect(tick, self.reader.rgb(),
            self.reader.dimensions(), self.reader.insets(), [num, den], self.target_aspect)?;
        Ok(true)
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    #[test]
    fn file_to_camera_pixels_seek_and_eof() {
        let mut file = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n".to_vec();
        for y in [16, 235] {
            file.extend_from_slice(b"FRAME\n");
            file.extend_from_slice(&[y, y, y, y, 128, 128]);
        }
        let reader = crate::playback::Y4mReader::new(std::io::Cursor::new(file), 18).unwrap();
        let mut source = Y4mCameraSource::new(reader);
        let output = LatestFrame::new(2, 2, 16).unwrap();
        let mut pixels = [0; 16];
        for (sequence, media, expected) in [
            (0, 0, 0),
            (1, 40_000_000, 255),
            (2, 400_000_000, 255),
            (3, 0, 0),
        ] {
            assert!(
                source
                    .publish(
                        CameraTick {
                            sequence,
                            host_time_ns: sequence + 1,
                            media_time_ns: media
                        },
                        &output
                    )
                    .unwrap()
            );
            output.copy_latest(None, &mut pixels).unwrap();
            assert_eq!(&pixels[..4], &[expected, expected, expected, 255]);
        }
    }
}

#[cfg(test)]
mod native_source_tests {
    use super::*;
    #[test]
    fn fractional_frame_boundary_seek_and_eof_are_exact() {
        let mut file = b"YUV4MPEG2 W2 H2 F30000:1001 Ip C420jpeg\n".to_vec();
        for y in [16, 235] {
            file.extend_from_slice(b"FRAME\n");
            file.extend_from_slice(&[y, y, y, y, 128, 128]);
        }
        let reader =
            crate::playback_native::NativeReader::new(std::io::Cursor::new(file), 18).unwrap();
        let mut source = NativeCameraSource::new(reader);
        let output = LatestFrame::new(2, 2, 16).unwrap();
        let mut pixels = [0; 16];
        for (sequence, (media, expected)) in [
            (0, 0),
            (33_366_666, 0),
            (33_366_667, 255),
            (999_999_999, 255),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let tick = CameraTick {
                sequence: sequence as u64,
                host_time_ns: sequence as u64 + 1,
                media_time_ns: media,
            };
            assert!(source.publish(tick, &output).unwrap());
            assert_eq!(output.copy_latest(None, &mut pixels).unwrap(), Some(tick));
            for pixel in pixels.as_chunks::<4>().0.iter() {
                assert_eq!(pixel, &[expected, expected, expected, 255]);
            }
        }
    }
}

/// Nearest-neighbour BGRA resize with centered opaque-black letterboxing.
/// Uses only caller-provided buffers; geometry is bounded before arithmetic.
pub fn fit_bgra(
    input: &[u8],
    source: [usize; 2],
    output: &mut [u8],
    target: [usize; 2],
) -> Result<()> {
    fit_bgra_aspect(input, source, [1, 1], output, target)
}
/// Fit packed samples using the source pixel aspect ratio; output pixels are square.
pub fn fit_bgra_aspect(
    input: &[u8], source: [usize; 2], pixel_aspect: [u32; 2],
    output: &mut [u8], target: [usize; 2],
) -> Result<()> {
    if pixel_aspect.contains(&0) { return Err(invalid("invalid camera pixel aspect")); }
    if source.into_iter().chain(target).any(|n| n == 0 || n > 4096)
        || input.len() != source[0] * source[1] * 4
        || output.len() != target[0] * target[1] * 4
    {
        return Err(invalid("invalid BGRA scaling geometry"));
    }
    let [sw, sh] = source;
    let [tw, th] = target;
    let display_w = sw as u128 * u128::from(pixel_aspect[0]);
    let display_h = sh as u128 * u128::from(pixel_aspect[1]);
    let (w, h) = if display_w * th as u128 > display_h * tw as u128 {
        (tw, (display_h * tw as u128 / display_w).max(1) as usize)
    } else {
        ((display_w * th as u128 / display_h).max(1) as usize, th)
    };
    for pixel in output.as_chunks_mut::<4>().0.iter_mut() {
        pixel.copy_from_slice(&[0, 0, 0, 255]);
    }
    let (left, top) = ((tw - w) / 2, (th - h) / 2);
    for y in 0..h {
        for x in 0..w {
            let src = ((y * sh / h) * sw + x * sw / w) * 4;
            let dst = ((top + y) * tw + left + x) * 4;
            output[dst..dst + 4].copy_from_slice(&input[src..src + 4]);
        }
    }
    Ok(())
}
#[cfg(test)]
mod fit_tests {
    use super::*;
    #[test]
    fn letterbox_preserves_aspect_pixels_and_opaque_borders() {
        let input = [1, 2, 3, 255, 4, 5, 6, 255];
        let mut output = [9; 64];
        fit_bgra(&input, [2, 1], &mut output, [4, 4]).unwrap();
        let row = [1, 2, 3, 255, 1, 2, 3, 255, 4, 5, 6, 255, 4, 5, 6, 255];
        assert_eq!(&output[16..32], &row);
        assert_eq!(&output[32..48], &row);
        for pixel in output[..16]
            .as_chunks::<4>().0.iter()
            .chain(output[48..].as_chunks::<4>().0.iter())
        {
            assert_eq!(pixel, &[0, 0, 0, 255]);
        }
        assert!(fit_bgra(&input, [0, 1], &mut output, [4, 4]).is_err());
        assert!(fit_bgra(&input, [usize::MAX, 1], &mut output, [4, 4]).is_err());
    }
}

#[cfg(test)]
mod crop_tests {
    use super::*;
    #[test]
    fn crop_copies_exact_rgb_pixels_without_a_second_frame_buffer() {
        let output = LatestFrame::new(2, 2, 16).unwrap();
        let rgb: Vec<u8> = (0..48).collect();
        let tick = CameraTick { sequence: 0, host_time_ns: 1, media_time_ns: 0 };
        output.publish_rgb_cropped(tick, &rgb, [4,4], [1,1,1,1]).unwrap();
        let mut actual = [0;16];
        output.copy_latest(None, &mut actual).unwrap().unwrap();
        assert_eq!(actual, [17,16,15,255,20,19,18,255,29,28,27,255,32,31,30,255]);
        assert!(output.publish_rgb_cropped(tick, &rgb[..47], [4,4], [1,1,1,1]).is_err());
        assert!(output.publish_rgb_cropped(tick, &rgb, [4,4], [0,0,0,0]).is_err());
        assert!(visible_dimensions([4,4],[4,0,0,0]).is_err());
        assert!(visible_dimensions([4,4],[u32::MAX,0,u32::MAX,0]).is_err());
        let mut unchanged = [0;16];
        output.copy_latest(None, &mut unchanged).unwrap();
        assert_eq!(unchanged, actual);
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    #[test]
    fn loops_at_eof_and_preserves_original_camera_clock() {
        let mut file = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n".to_vec();
        for y in [16,235] { file.extend_from_slice(b"FRAME\n"); file.extend_from_slice(&[y,y,y,y,128,128]); }
        let reader = crate::playback::Y4mReader::new(std::io::Cursor::new(file),18).unwrap();
        let mut source = NativeCameraSource::new(crate::playback_native::NativeReader::Y4m(reader)).with_end_behavior(CameraEndBehavior::Loop);
        let output = LatestFrame::new(2,2,16).unwrap();
        let mut pixels = [0;16];
        for (sequence,(media,expected)) in [(0,0),(40_000_000,255),(80_000_000,0),(120_000_000,255),(400_000_000,0),(440_000_000,255),(0,0)].into_iter().enumerate() {
            let tick = CameraTick { sequence:sequence as u64, host_time_ns:sequence as u64+1, media_time_ns:media };
            assert!(source.publish(tick,&output).unwrap());
            assert_eq!(output.copy_latest(None,&mut pixels).unwrap(),Some(tick));
            assert_eq!(&pixels[..4],&[expected,expected,expected,255]);
        }
    }
}
