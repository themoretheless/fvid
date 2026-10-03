//! Integer cross-fade of matching decoded frames, without a media backend.
use crate::{owned_frame::GeometryFrame, owned_overlay::validate_frame};

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
