//! Granule-qualified Vorbis packet presentation; no container dependency.
use crate::vorbis::{Mode, PacketBits, PacketMode};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTime {
    pub pts: i64,
    pub duration: i64,
}
pub struct Timeline {
    pub frames: Vec<FrameTime>,
    pub end: i64,
}
/// First packet only primes overlap. A page granule anchors the end of its last
/// completed packet; the EOS granule may shorten that packet's output tail.
pub fn build(
    blocks: [u8; 2],
    modes: &[Mode],
    packets: &[(&[u8], Option<i64>)],
    ended: bool,
) -> Result<Timeline> {
    if packets.is_empty() || blocks.iter().any(|e| !(6..=13).contains(e)) || blocks[0] > blocks[1] {
        return Err(invalid("invalid Vorbis timeline geometry"));
    }
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(packets.len())
        .map_err(|_| invalid("cannot allocate Vorbis timeline"))?;
    let (mut previous, mut cumulative) = (None, 0i64);
    for &(packet, _) in packets {
        let header = PacketMode::parse(&mut PacketBits::new(packet), modes)?;
        let n = 1i64 << blocks[usize::from(modes[header.mode].long)];
        let duration = previous.map_or(0, |p| (p + n) / 4);
        frames.push(FrameTime {
            pts: cumulative,
            duration,
        });
        cumulative = cumulative
            .checked_add(duration)
            .ok_or_else(|| invalid("Vorbis sample clock overflow"))?;
        previous = Some(n);
    }
    let final_index = packets.len() - 1;
    let anchor = packets.iter().enumerate().find_map(|(i, (_, g))| {
        if ended && i == final_index {
            None
        } else {
            g.map(|g| (i, g))
        }
    });
    let offset = match anchor {
        Some((i, granule)) => granule
            .checked_sub(frames[i].pts + frames[i].duration)
            .ok_or_else(|| invalid("Vorbis granule offset overflow"))?,
        None => 0,
    };
    if let Some((index, granule)) = anchor {
        if granule < 0 || offset < -frames.get(1).map_or(0, |f| f.duration) {
            return Err(invalid("Vorbis initial trim exceeds first overlap"));
        }
        for (i, (_, g)) in packets.iter().enumerate().skip(index) {
            if ended && i == final_index {
                break;
            }
            if let Some(granule) = g {
                let expected = frames[i]
                    .pts
                    .checked_add(frames[i].duration)
                    .and_then(|n| n.checked_add(offset))
                    .ok_or_else(|| invalid("Vorbis granule clock overflow"))?;
                if *granule != expected {
                    return Err(invalid("inconsistent Vorbis page granule"));
                }
            }
        }
    }
    let decoded_end = cumulative
        .checked_add(offset)
        .ok_or_else(|| invalid("Vorbis presentation clock overflow"))?;
    let end = if ended {
        packets[final_index].1.unwrap_or(decoded_end)
    } else {
        decoded_end
    };
    let final_start = frames[final_index]
        .pts
        .checked_add(offset)
        .ok_or_else(|| invalid("Vorbis final packet clock overflow"))?;
    if end < 0 || end > decoded_end || end < final_start.max(0) {
        return Err(invalid("Vorbis EOS trim exceeds final packet"));
    }
    for frame in &mut frames {
        frame.pts = frame
            .pts
            .checked_add(offset)
            .ok_or_else(|| invalid("Vorbis packet timestamp overflow"))?;
    }
    Ok(Timeline { frames, end })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_modes_use_overlap_durations_and_granule_offsets() {
        let modes = [
            Mode {
                long: false,
                mapping: 0,
            },
            Mode {
                long: false,
                mapping: 0,
            },
            Mode {
                long: true,
                mapping: 0,
            },
        ];
        let a = [0u8];
        let b = [4u8];
        for offset in [-17, 0, 23] {
            let packets = [
                (&a[..], None),
                (&b[..], Some(80 + offset)),
                (&a[..], None),
                (&b[..], None),
                (&b[..], None),
                (&a[..], Some(411 + offset)),
            ];
            let timeline = build([6, 8], &modes, &packets, true).unwrap();
            assert_eq!(timeline.end, 411 + offset);
            assert_eq!(
                timeline
                    .frames
                    .iter()
                    .map(|f| f.duration)
                    .collect::<Vec<_>>(),
                vec![0, 80, 80, 80, 128, 80]
            );
            assert_eq!(
                timeline.frames.iter().map(|f| f.pts).collect::<Vec<_>>(),
                vec![
                    offset,
                    offset,
                    80 + offset,
                    160 + offset,
                    240 + offset,
                    368 + offset
                ]
            );
        }
    }
}
