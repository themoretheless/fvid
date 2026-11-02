// Ac3::parse_mode builds this index in increasing native sample order.
fn raw_frame_window(frames: &[Frame], source_pts: i64) -> usize {
    frames.iter().find(|at| at.pts as i64 == source_pts)
        .map(|at| at.samples as usize).unwrap_or(SAMPLES_PER_FRAME as usize)
}
