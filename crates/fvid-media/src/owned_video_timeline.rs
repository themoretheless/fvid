//! Shared rate-one MP4 video presentation edits, in track ticks.
use fvid_control::error::{Error, Result, invalid, unsupported};
#[derive(Clone, Copy, Debug)]
pub struct PlaybackEdit {
    pub media_start: i64,
    pub media_end: i64,
    pub movie_start: i64,
    pub movie_end: i64,
}
/// Leading empty edits retain start-at-first-picture playback. Interior empty
/// edits require a blank-frame policy and remain an explicit capability refusal.
pub fn map_edits(
    edits: impl IntoIterator<Item = (u64, i64)>,
    track_scale: u32,
    movie_scale: u32,
) -> Result<Vec<PlaybackEdit>> {
    let mut timeline = 0u128;
    let mut movie_start = 0i64;
    let mut result = Vec::new();
    for (duration, media_start) in edits.into_iter().skip_while(|(_, start)| *start < 0) {
        if media_start < 0 {
            return Err(unsupported(
                "empty MP4 edit inside playback is not implemented",
            ));
        }
        if movie_scale == 0 || track_scale == 0 {
            return Err(invalid("MP4 movie or track timescale is zero"));
        }
        timeline = timeline
            .checked_add(u128::from(duration))
            .ok_or_else(|| invalid("MP4 edit duration overflow"))?;
        let movie_end = i64::try_from(
            timeline
                .checked_mul(u128::from(track_scale))
                .ok_or_else(|| invalid("MP4 edit duration overflow"))?
                .div_ceil(u128::from(movie_scale)),
        )
        .map_err(|_| invalid("MP4 edit duration overflow"))?;
        let duration = movie_end
            .checked_sub(movie_start)
            .ok_or_else(|| invalid("MP4 edit duration overflow"))?;
        if duration <= 0 {
            return Err(invalid("empty MP4 playback edit"));
        }
        let media_end = media_start
            .checked_add(duration)
            .ok_or_else(|| invalid("MP4 edit endpoint overflow"))?;
        result
            .try_reserve(1)
            .map_err(|_| Error::Invalid("MP4 edit allocation failed".into()))?;
        result.push(PlaybackEdit {
            media_start,
            media_end,
            movie_start,
            movie_end,
        });
        movie_start = movie_end;
    }
    Ok(result)
}
/// Each overlapping range displays its own clipped copy of a decoded frame.
/// Repeated edits therefore count repeatedly; preroll is decoded but not shown.
pub fn appearances(edits: &[PlaybackEdit], start: i64, duration: i64) -> Result<u64> {
    let end = start
        .checked_add(duration)
        .ok_or_else(|| invalid("video timestamp overflow"))?;
    if duration <= 0 {
        return Err(invalid("invalid video duration"));
    }
    if edits.is_empty() {
        return Ok(u64::from(end > 0));
    }
    edits.iter().try_fold(0u64, |count, edit| {
        count
            .checked_add(u64::from(end > edit.media_start && start < edit.media_end))
            .ok_or_else(|| invalid("video frame count overflow"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_fractional_rounding_does_not_drift() {
        let edits = map_edits([(1, 10), (1, 10), (1, 10)], 3, 2).unwrap();
        assert_eq!(
            edits
                .iter()
                .map(|v| (v.movie_start, v.movie_end, v.media_start, v.media_end))
                .collect::<Vec<_>>(),
            vec![(0, 2, 10, 12), (2, 3, 10, 11), (3, 5, 10, 12)]
        );
    }
    #[test]
    fn leading_empty_edits_and_half_open_media_boundaries() {
        let edits = map_edits([(99, -1), (2, 10), (2, 10)], 1, 1).unwrap();
        assert_eq!(edits[0].movie_start, 0);
        assert_eq!(appearances(&edits, 9, 1).unwrap(), 0);
        assert_eq!(appearances(&edits, 9, 2).unwrap(), 2);
        assert_eq!(appearances(&edits, 12, 1).unwrap(), 0);
        assert_eq!(appearances(&edits, 11, 1).unwrap(), 2);
        assert_eq!(appearances(&[], -1, 1).unwrap(), 0);
        assert_eq!(appearances(&[], -1, 2).unwrap(), 1);
    }
    #[test]
    fn impossible_clocks_endpoints_and_empty_ranges_are_errors() {
        assert!(map_edits([(1, 0)], 0, 1).is_err());
        assert!(map_edits([(1, 0)], 1, 0).is_err());
        assert!(map_edits([(0, 0)], 1, 1).is_err());
        assert!(map_edits([(u64::MAX, 0)], u32::MAX, 1).is_err());
        assert!(map_edits([(1, i64::MAX)], 1, 1).is_err());
        assert!(appearances(&[], i64::MAX, 1).is_err());
        assert!(appearances(&[], 0, 0).is_err());
        assert!(matches!(
            map_edits([(1, 0), (1, -1)], 1, 1),
            Err(Error::Unsupported(_))
        ));
    }
}
