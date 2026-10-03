//! Integer cross-fade of matching decoded frames, without a media backend.
use crate::owned_y4m::{Header, line};
use crate::{owned_frame::GeometryFrame, owned_overlay::validate_frame};
use std::io::BufRead;

/// Forward-only secondary video reader. Retains one frame and selects the most
/// recent frame on its own clock; callers decide how to handle a short source.
pub struct SecondaryReader<R> {
    input: R,
    header: Header,
    current: Option<GeometryFrame>,
    next_index: u64,
    last_target: Option<u64>,
    eof: bool,
    failed: bool,
}
impl<R: BufRead> SecondaryReader<R> {
    pub fn new(mut input: R, primary: &Header) -> Result<Self, String> {
        let mut bytes = Vec::new();
        if !line(&mut input, &mut bytes)? {
            return Err("empty cross-fade secondary source".into());
        }
        let header = Header::parse(&bytes)?;
        header.frame_rate()?;
        if header.width != primary.width
            || header.height != primary.height
            || header.format != primary.format
            || header.depth() != primary.depth()
            || header.full_range()? != primary.full_range()?
        {
            return Err("cross-fade inputs require matching geometry, format and range".into());
        }
        Ok(Self {
            input,
            header,
            current: None,
            next_index: 0,
            last_target: None,
            eof: false,
            failed: false,
        })
    }
    pub fn frame_at(&mut self, pts_ns: u64) -> Result<Option<&GeometryFrame>, String> {
        if self.failed {
            return Err("cross-fade secondary reader requires reset after error".into());
        }
        if let Err(error) = self.advance(pts_ns) {
            self.failed = true;
            self.current = None;
            return Err(error);
        }
        Ok(self.current.as_ref())
    }
    fn advance(&mut self, pts_ns: u64) -> Result<(), String> {
        if self.last_target.is_some_and(|last| pts_ns < last) {
            return Err("cross-fade secondary clock cannot rewind".into());
        }
        self.last_target = Some(pts_ns);
        let [num, den] = self.header.frame_rate()?;
        let mut marker = Vec::new();
        while !self.eof {
            // Compare rationals directly, without rounding the secondary PTS.
            if u128::from(self.next_index) * den as u128 * 1_000_000_000
                > u128::from(pts_ns) * num as u128
            {
                break;
            }
            if !line(&mut self.input, &mut marker)? {
                self.eof = true;
                break;
            }
            if marker != b"FRAME\n" && !marker.starts_with(b"FRAME ") {
                return Err("expected cross-fade Y4M FRAME marker".into());
            }
            let size = self.header.frame_len()?;
            if self.current.is_none() {
                let (sx, sy) = self.header.format.subsampling();
                self.current = Some(GeometryFrame {
                    width: self.header.width,
                    height: self.header.height,
                    subsampling: Some([sx, sy]),
                    data: crate::owned_frame::buffer(size)?,
                });
            }
            self.input
                .read_exact(&mut self.current.as_mut().unwrap().data)
                .map_err(|e| format!("cross-fade secondary payload: {e}"))?;
            self.next_index = self
                .next_index
                .checked_add(1)
                .ok_or("cross-fade frame count overflow")?;
        }
        Ok(())
    }
    pub fn exhausted(&self) -> bool {
        self.eof
    }
    /// Apply a transition on the primary presentation clock. A missing first
    /// secondary frame is an error; a nonempty short source repeats its tail.
    pub fn apply(
        &mut self,
        frame: &mut GeometryFrame,
        depth: u8,
        pts_ns: u64,
        timeline: FadeTimeline,
    ) -> Result<(), String> {
        let (secondary_ns, elapsed, duration) = match timeline.phase(pts_ns) {
            Phase::Primary => return Ok(()),
            Phase::Blend {
                secondary_ns,
                elapsed_ns,
                duration_ns,
            } => (secondary_ns, elapsed_ns, duration_ns),
            Phase::Secondary { secondary_ns } => (secondary_ns, 1, 1),
        };
        let next = self
            .frame_at(secondary_ns)?
            .ok_or("cross-fade secondary source has no frames")?;
        fade(frame, next, depth, elapsed, duration)
    }
}

/// Exact nanosecond timeline for a fade into a secondary source starting at zero.
#[derive(Clone, Copy, Debug)]
pub struct FadeTimeline {
    offset_ns: u64,
    duration_ns: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Primary,
    Blend {
        secondary_ns: u64,
        elapsed_ns: u64,
        duration_ns: u64,
    },
    Secondary {
        secondary_ns: u64,
    },
}
impl FadeTimeline {
    pub fn new(offset_us: i64, duration_us: i64) -> Result<Self, String> {
        if offset_us < 0 || duration_us <= 0 {
            return Err("cross-fade requires nonnegative offset and positive duration".into());
        }
        let offset_ns = (offset_us as u64)
            .checked_mul(1000)
            .ok_or("cross-fade offset overflow")?;
        let duration_ns = (duration_us as u64)
            .checked_mul(1000)
            .ok_or("cross-fade duration overflow")?;
        offset_ns
            .checked_add(duration_ns)
            .ok_or("cross-fade endpoint overflow")?;
        Ok(Self {
            offset_ns,
            duration_ns,
        })
    }
    pub fn phase(self, pts_ns: u64) -> Phase {
        let Some(elapsed) = pts_ns.checked_sub(self.offset_ns) else {
            return Phase::Primary;
        };
        if elapsed >= self.duration_ns {
            Phase::Secondary {
                secondary_ns: elapsed,
            }
        } else {
            Phase::Blend {
                secondary_ns: elapsed,
                elapsed_ns: elapsed,
                duration_ns: self.duration_ns,
            }
        }
    }
}

/// Blend toward `next` by `elapsed / duration`, rounding ties upward.
/// Inputs must share colour encoding and range. Both frames are validated
/// before mutation; frame timing and colour conversion belong to the caller.
pub fn fade(
    frame: &mut GeometryFrame,
    next: &GeometryFrame,
    depth: u8,
    elapsed: u64,
    duration: u64,
) -> Result<(), String> {
    if duration == 0 || elapsed > duration {
        return Err("invalid cross-fade interval".into());
    }
    validate_frame(frame, depth)?;
    validate_frame(next, depth)?;
    if frame.width != next.width
        || frame.height != next.height
        || frame.subsampling != next.subsampling
    {
        return Err("cross-fade requires matching frame geometry".into());
    }
    let blend = |a: u16, b: u16| -> u16 {
        ((u128::from(a) * u128::from(duration - elapsed)
            + u128::from(b) * u128::from(elapsed)
            + u128::from(duration / 2))
            / u128::from(duration)) as u16
    };
    if depth == 8 {
        for (a, b) in frame.data.iter_mut().zip(&next.data) {
            *a = blend(u16::from(*a), u16::from(*b)) as u8;
        }
    } else {
        for (a, b) in frame
            .data
            .chunks_exact_mut(2)
            .zip(next.data.chunks_exact(2))
        {
            let value = blend(
                u16::from_le_bytes([a[0], a[1]]),
                u16::from_le_bytes([b[0], b[1]]),
            );
            a.copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_transition_blends_on_primary_clock_then_uses_secondary_pixels() {
        let primary = Header::parse(b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n").unwrap();
        let mut bytes = b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n".to_vec();
        for value in [100, 200] {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[value; 6]);
        }
        let mut reader = SecondaryReader::new(std::io::Cursor::new(bytes), &primary).unwrap();
        let timeline = FadeTimeline::new(100000, 400000).unwrap();
        for (pts, expected) in [(0, 0), (100000000, 0), (300000000, 50), (500000000, 200)] {
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: vec![0; 6],
            };
            reader.apply(&mut frame, 8, pts, timeline).unwrap();
            assert_eq!(frame.data, [expected; 6]);
        }
    }
    #[test]
    fn truncated_secondary_frame_poison_is_specific_and_persistent() {
        let primary = Header::parse(b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n").unwrap();
        let source =
            include_bytes!("../../../tests/fixtures/playback-errors/xfade-secondary-truncated.y4m");
        let mut reader = SecondaryReader::new(std::io::Cursor::new(source), &primary).unwrap();
        assert!(
            reader
                .frame_at(0)
                .unwrap_err()
                .contains("cross-fade secondary payload")
        );
        assert!(
            reader
                .frame_at(0)
                .unwrap_err()
                .contains("requires reset after error")
        );
        assert!(reader.current.is_none());
    }
    #[test]
    fn secondary_reader_selects_exact_fractional_clock_and_retains_last_frame() {
        let primary = Header::parse(b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n").unwrap();
        let mut bytes = b"YUV4MPEG2 W2 H2 F3:1 Ip A1:1 C420\n".to_vec();
        for value in [10, 20, 30] {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[value; 6]);
        }
        let mut reader = SecondaryReader::new(std::io::Cursor::new(bytes), &primary).unwrap();
        for (pts, value) in [
            (0, 10),
            (333333333, 10),
            (333333334, 20),
            (666666667, 30),
            (1000000000, 30),
        ] {
            assert_eq!(reader.frame_at(pts).unwrap().unwrap().data, [value; 6]);
        }
        assert!(reader.exhausted());
        assert!(reader.frame_at(0).is_err());
    }
    #[test]
    fn timeline_preserves_submicrosecond_pts_and_secondary_clock() {
        let timeline = FadeTimeline::new(2, 3).unwrap();
        assert_eq!(timeline.phase(1999), Phase::Primary);
        assert_eq!(
            timeline.phase(2000),
            Phase::Blend {
                secondary_ns: 0,
                elapsed_ns: 0,
                duration_ns: 3000
            }
        );
        assert_eq!(
            timeline.phase(3499),
            Phase::Blend {
                secondary_ns: 1499,
                elapsed_ns: 1499,
                duration_ns: 3000
            }
        );
        assert_eq!(
            timeline.phase(5000),
            Phase::Secondary { secondary_ns: 3000 }
        );
        assert_eq!(
            timeline.phase(u64::MAX),
            Phase::Secondary {
                secondary_ns: u64::MAX - 2000
            }
        );
        assert!(FadeTimeline::new(-1, 3).is_err());
        assert!(FadeTimeline::new(0, 0).is_err());
        assert!(FadeTimeline::new(i64::MAX, 1).is_err());
        assert!(FadeTimeline::new((u64::MAX / 1000) as i64, 1).is_err());
    }
    fn rgb(values: [u8; 3]) -> GeometryFrame {
        GeometryFrame {
            width: 1,
            height: 1,
            subsampling: None,
            data: values.to_vec(),
        }
    }
    #[test]
    fn endpoints_midpoint_and_full_timestamp_range() {
        let next = rgb([255, 100, 1]);
        for (elapsed, duration, expected) in [
            (0, 2, [0, 0, 0]),
            (1, 2, [128, 50, 1]),
            (2, 2, [255, 100, 1]),
            (u64::MAX, u64::MAX, [255, 100, 1]),
        ] {
            let mut frame = rgb([0, 0, 0]);
            fade(&mut frame, &next, 8, elapsed, duration).unwrap();
            assert_eq!(frame.data, expected);
        }
    }
    #[test]
    fn all_planar_depths_preserve_samples_and_fail_atomically() {
        for depth in 8..=16 {
            let maximum = ((1u32 << depth) - 1) as u16;
            let bytes = |value: u16| {
                if depth == 8 {
                    vec![value as u8; 3]
                } else {
                    value.to_le_bytes().repeat(3)
                }
            };
            let mut frame = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([2, 2]),
                data: bytes(0),
            };
            let mut next = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([2, 2]),
                data: bytes(maximum),
            };
            fade(&mut frame, &next, depth, 1, 2).unwrap();
            assert_eq!(frame.data, bytes(maximum / 2 + 1));
            let saved = frame.data.clone();
            next.data.pop();
            assert!(fade(&mut frame, &next, depth, 1, 2).is_err());
            assert_eq!(frame.data, saved);
            let copy = frame_copy(&frame);
            assert!(fade(&mut frame, &copy, depth, 0, 0).is_err());
        }
    }
    fn frame_copy(frame: &GeometryFrame) -> GeometryFrame {
        GeometryFrame {
            width: frame.width,
            height: frame.height,
            subsampling: frame.subsampling,
            data: frame.data.clone(),
        }
    }
}
