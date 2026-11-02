// Ac3::parse_mode constructs an ordered, non-wrapping native sample clock.
fn raw_frame_window(frames: &[Frame], source_pts: i64) -> usize {
    // This cast preserves the original `frame.pts as i64 == source_pts` key,
    // including the signed-coordinate edge cases. Select the first equal key.
    let target = source_pts as u64;
    let index = frames.partition_point(|frame| frame.pts < target);
    frames.get(index).filter(|frame| frame.pts == target)
        .map_or(SAMPLES_PER_FRAME as usize, |frame| frame.samples as usize)
}

#[cfg(test)]
mod window_lookup_tests {
    use super::{Frame, SAMPLES_PER_FRAME, raw_frame_window};
    fn frame(pts: u64, samples: u32) -> Frame {
        Frame { start: 0, size: 0, pts, rate: 48000, channels: 1, samples }
    }
    #[test]
    fn ordered_window_lookup_retains_first_match_fallback_and_signed_keys() {
        let sets = [vec![], vec![frame(0, 256)],
            vec![frame(0, 256), frame(256, 512), frame(768, 768), frame(1536, 1536)],
            vec![frame(0, 256), frame(0, 512), frame(3, 768), frame(3, 1536)],
            vec![frame(0, 256), frame(i64::MAX as u64, 512),
                frame(1u64 << 63, 768), frame(u64::MAX, 256)]];
        for frames in sets {
            let queries = [i64::MIN, -1, 0, 1, 2, 3, 255, 256, 257, 768, 1536, i64::MAX];
            for pts in queries.into_iter().chain(frames.iter().map(|frame| frame.pts as i64)) {
                let expected = frames.iter().find(|frame| frame.pts as i64 == pts)
                    .map_or(SAMPLES_PER_FRAME as usize, |frame| frame.samples as usize);
                assert_eq!(raw_frame_window(&frames, pts), expected, "{frames:?}: {pts}");
            }
        }
    }
}
