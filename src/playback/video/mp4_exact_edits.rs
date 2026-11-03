//! Exact movie/media edit grid for fractional NativeReader presentation windows.
//! The raw MP4 source and the public reader variant retain their media clocks.
use crate::container::mp4::Track;
use crate::playback_mp4::{Mp4VideoReader, VideoFrame};
use crate::{Result, invalid, unsupported};
use std::{
    io::{Read, Seek},
    time::Duration,
};

#[derive(Clone, Copy)]
struct Window {
    media: i128,
    end: i128,
    movie: i128,
    movie_end: i128,
}
pub(crate) struct ExactVideoEdits {
    windows: Vec<Window>,
    scale: u64,
    factor: i128,
    index: usize,
    last: Option<(i128, i128)>,
    resync: bool,
}
impl ExactVideoEdits {
    pub(crate) fn new(track: &Track, movie_scale: u32, budget: usize) -> Result<Option<Self>> {
        // Open edits resolve directly in media ticks. Let the legacy path do
        // that potentially large endpoint scan once, rather than scanning here
        // and then again when no secondary grid is needed.
        if track.edits.is_empty() || track.has_open_edit() {
            return Ok(None);
        }
        let clock = track.resolved_edit_clock(movie_scale)?;
        let edits = clock.edits();
        let visible = edits.iter().skip_while(|e| e.media_time < 0);
        if !visible.clone().any(|e| {
            u128::from(e.duration) * u128::from(track.timescale) % u128::from(clock.timescale) != 0
        }) {
            return Ok(None);
        }
        let mut a = u64::from(track.timescale);
        let mut b = u64::from(clock.timescale);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let scale = u64::from(track.timescale) / a * u64::from(clock.timescale);
        let factor = i128::from(scale / u64::from(track.timescale));
        let movie_factor = i128::from(scale / u64::from(clock.timescale));
        let count = visible.clone().count();
        let bytes = count
            .checked_mul(std::mem::size_of::<Window>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| invalid("MP4 exact edit clock allocation overflow"))?;
        if bytes > budget {
            return Err(invalid("MP4 exact edit clock exceeds playback budget"));
        }
        let mut windows = Vec::new();
        windows
            .try_reserve_exact(count)
            .map_err(|_| invalid("MP4 exact edit clock allocation failed"))?;
        let mut movie = 0i128;
        for edit in visible {
            if edit.media_time < 0 {
                return Err(unsupported(
                    "empty MP4 edit inside playback is not implemented",
                ));
            }
            if edit.duration == 0 {
                return Err(invalid("empty MP4 playback edit"));
            }
            let duration = i128::from(edit.duration) * movie_factor;
            if (duration as u128) * 1_000_000_000 < u128::from(scale) {
                return Err(unsupported(
                    "MP4 edit interval is below nanosecond playback precision",
                ));
            }
            let end = movie
                .checked_add(duration)
                .ok_or_else(|| invalid("MP4 edit duration overflow"))?;
            // Preserve the legacy signed track-clock endpoint bounds without
            // rounding the stored movie or media intervals to that grid.
            i64::try_from((end + factor - 1) / factor)
                .map_err(|_| invalid("MP4 edit duration overflow"))?;
            let media = i128::from(edit.media_time) * factor;
            let media_end = media + duration;
            i64::try_from((media_end + factor - 1) / factor)
                .map_err(|_| invalid("MP4 edit endpoint overflow"))?;
            windows.push(Window {
                media,
                end: media_end,
                movie,
                movie_end: end,
            });
            movie = end;
        }
        Ok(Some(Self {
            windows,
            scale,
            factor,
            index: 0,
            last: None,
            resync: false,
        }))
    }
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.windows.capacity() * std::mem::size_of::<Window>()
    }
    pub(crate) fn reset(&mut self) {
        self.index = 0;
        self.last = None;
        self.resync = false;
    }
    fn time(&self, ticks: i128) -> Option<Duration> {
        let n = u128::try_from(ticks).ok()?;
        let d = u128::from(self.scale);
        Some(Duration::new(
            u64::try_from(n / d).ok()?,
            ((n % d) * 1_000_000_000 / d) as u32,
        ))
    }
    pub(crate) fn duration(&self) -> Option<Duration> {
        self.time(self.windows.last()?.movie_end)
    }
    fn nanos(&self, ticks: i128) -> Option<u128> {
        let n = u128::try_from(ticks).ok()?;
        let d = u128::from(self.scale);
        (n / d)
            .checked_mul(1_000_000_000)?
            .checked_add((n % d) * 1_000_000_000 / d)
    }
    pub(crate) fn interval(&self) -> Option<(u128, u128, u32)> {
        let (start, end) = self.last?;
        if let Ok(scale) = u32::try_from(self.scale) {
            Some((start as u128, end as u128, scale))
        } else {
            Some((self.nanos(start)?, self.nanos(end)?, 1_000_000_000))
        }
    }
    pub(crate) fn legacy_interval(&self) -> Option<(i64, i64)> {
        let (start, end) = self.last?;
        Some((
            i64::try_from(start / self.factor).ok()?,
            i64::try_from((end + self.factor - 1) / self.factor).ok()?,
        ))
    }
    pub(crate) fn period_nanos(&self) -> Result<i128> {
        let (start, end) = self
            .last
            .ok_or_else(|| invalid("missing MP4 edit interval"))?;
        if self.nanos(end - start) == Some(0) {
            return Err(unsupported(
                "MP4 edit interval is below nanosecond playback precision",
            ));
        }
        let ns = if u32::try_from(self.scale).is_ok() {
            self.nanos(end - start)
        } else {
            self.nanos(end)
                .zip(self.nanos(start))
                .and_then(|(b, a)| b.checked_sub(a))
        };
        ns.and_then(|n| i128::try_from(n).ok())
            .ok_or_else(|| invalid("video edit duration overflow"))
    }
    pub(crate) fn covers(&self, target: i128) -> bool {
        self.last.is_some_and(|(_, end)| end > target)
    }
    pub(crate) fn starts_after(&self, target: i128) -> bool {
        self.last.is_some_and(|(start, _)| start > target)
    }
    pub(crate) fn seek<R: Read + Seek>(
        &mut self,
        source: &mut Mp4VideoReader<R>,
        target: Duration,
    ) -> Result<(i128, bool)> {
        let end = self
            .windows
            .last()
            .ok_or_else(|| invalid("empty MP4 playback edit"))?
            .movie_end;
        let d = u128::from(self.scale);
        let end_u = end as u128;
        // Compare whole seconds before multiplying a possibly Duration::MAX
        // request by the common grid. A request inside the program is bounded.
        let seconds = u128::from(target.as_secs());
        let past = seconds > end_u / d
            || (seconds == end_u / d
                && u128::from(target.subsec_nanos()) * d >= (end_u % d) * 1_000_000_000);
        let wanted = if past {
            end - 1
        } else {
            i128::try_from(seconds * d + u128::from(target.subsec_nanos()) * d / 1_000_000_000)
                .map_err(|_| invalid("seek target overflow"))?
                .min(end - 1)
        };
        self.index = self
            .windows
            .partition_point(|w| w.movie_end <= wanted)
            .min(self.windows.len() - 1);
        let edit = self.windows[self.index];
        let media = (edit.media + wanted - edit.movie) / self.factor;
        source.seek_to_sync(i64::try_from(media).map_err(|_| invalid("seek target overflow"))?);
        self.last = None;
        self.resync = true;
        Ok((wanted, source.starts_at_first_sample() && self.index == 0))
    }
    pub(crate) fn cache_packets(&self, track: &Track) -> Vec<(u64, u64, Duration, Duration)> {
        track
            .samples
            .expanded()
            .unwrap_or(&[])
            .iter()
            .flat_map(|sample| {
                self.windows.iter().filter_map(move |w| {
                    let start = (i128::from(sample.pts) * self.factor).max(w.media);
                    let end = ((i128::from(sample.pts) + i128::from(sample.duration))
                        * self.factor)
                        .min(w.end);
                    if end <= start {
                        return None;
                    }
                    Some((
                        sample.offset,
                        sample.offset.saturating_add(u64::from(sample.size)),
                        self.time(start - w.media + w.movie)?,
                        self.time(end - w.media + w.movie)?,
                    ))
                })
            })
            .collect()
    }
    pub(crate) fn advance<R: Read + Seek>(
        &mut self,
        source: &mut Mp4VideoReader<R>,
    ) -> Result<Option<VideoFrame>> {
        let mut finished = self
            .last
            .is_some_and(|(_, end)| end >= self.windows[self.index].movie_end);
        let mut complete =
            self.last.is_none() && self.index == 0 && source.starts_at_first_sample();
        // NativeReader's target-specific seek recovery owns the initial
        // coverage check. Prefix recovery here is only for a new edit window.
        let mut retry = false;
        let mut saw_picture = false;
        loop {
            if finished {
                if self.index + 1 == self.windows.len() {
                    return Ok(None);
                }
                self.index += 1;
                source.seek_to_sync(
                    i64::try_from(self.windows[self.index].media / self.factor)
                        .map_err(|_| invalid("MP4 edit endpoint overflow"))?,
                );
                self.resync = true;
                finished = false;
                complete = source.starts_at_first_sample();
                retry = !complete;
                saw_picture = false;
            }
            let Some(frame) = source.read_frame()? else {
                if complete && !saw_picture {
                    return Ok(None);
                }
                if retry {
                    source.rewind();
                    self.resync = true;
                    complete = true;
                    retry = false;
                    saw_picture = false;
                    continue;
                }
                finished = true;
                continue;
            };
            let w = self.windows[self.index];
            let begin = i128::from(frame.presentation_time.ticks) * self.factor;
            let end = (i128::from(frame.presentation_time.ticks)
                + i128::from(frame.duration.ticks))
                * self.factor;
            saw_picture |= end > 0;
            if end <= w.media {
                continue;
            }
            if retry {
                retry = false;
                if begin > w.media {
                    drop(frame);
                    source.rewind();
                    self.resync = true;
                    complete = true;
                    saw_picture = false;
                    continue;
                }
            }
            if begin >= w.end {
                finished = true;
                continue;
            }
            let start = begin.max(w.media) - w.media + w.movie;
            let end = end.min(w.end) - w.media + w.movie;
            if end <= start || (!self.resync && self.last.is_some_and(|(_, old)| start < old)) {
                return Err(invalid(
                    "overlapping video presentation timestamps or invalid duration",
                ));
            }
            self.last = Some((start, end));
            self.resync = false;
            return Ok(Some(frame));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_edit_storage_is_admitted_before_allocating_the_program() {
        let source = include_bytes!(
            "../../../tests/fixtures/playback-errors/mp4-subtick-video-edits/two-tiny.mp4"
        );
        let reader = crate::container::mp4::Mp4Reader::open_indexed(
            std::io::Cursor::new(source),
            Default::default(),
        )
        .unwrap();
        let track = &reader.tracks()[0];
        assert!(
            ExactVideoEdits::new(track, reader.movie_timescale(), 0)
                .err()
                .unwrap()
                .to_string()
                .contains("playback budget")
        );
        let clock = ExactVideoEdits::new(track, reader.movie_timescale(), 4096)
            .unwrap()
            .unwrap();
        assert!(clock.bytes() <= 4096);
        assert_eq!(clock.duration(), Some(Duration::from_millis(2)));
        assert!(ExactVideoEdits::new(track, 0, 4096).is_err());
    }
}
