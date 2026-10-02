#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    pub presentation: std::ops::Range<u64>,
    /// None schedules silence; Some names the first source sample to decode.
    pub source_start: Option<u64>,
}
pub struct AudioTimeline {
    pub segments: Vec<Segment>,
    pub sample_frames: u64,
}
impl AudioTimeline {
    /// Movie boundaries round cumulatively; media starts must be sample aligned.
    pub fn new(
        edits: &[Edit],
        media_duration: u64,
        media_scale: u32,
        movie_scale: u32,
        rate: u32,
    ) -> Result<Self> {
        if media_scale == 0 || movie_scale == 0 || rate == 0 {
            return Err(invalid("zero audio timeline clock"));
        }
        let source_sample = |ticks: u64| -> Result<u64> {
            let value = u128::from(ticks) * u128::from(rate);
            if value % u128::from(media_scale) != 0 {
                return Err(invalid("MP4 audio timestamp is not aligned to a sample"));
            }
            u64::try_from(value / u128::from(media_scale))
                .map_err(|_| invalid("audio timeline overflow"))
        };
        let mut segments = Vec::new();
        segments
            .try_reserve_exact(edits.len().max(1))
            .map_err(|_| invalid("audio timeline allocation failed"))?;
        let mut sample_frames = 0;
        let mut movie_ticks = 0u128;
        if edits.is_empty() {
            sample_frames = source_sample(media_duration)?;
            segments.push(Segment {
                presentation: 0..sample_frames,
                source_start: Some(0),
            });
        } else {
            for edit in edits {
                movie_ticks = movie_ticks
                    .checked_add(u128::from(edit.duration))
                    .ok_or_else(|| invalid("audio timeline overflow"))?;
                let end = movie_ticks
                    .checked_mul(u128::from(rate))
                    .ok_or_else(|| invalid("audio timeline overflow"))?
                    .div_ceil(u128::from(movie_scale));
                let end = u64::try_from(end).map_err(|_| invalid("audio timeline overflow"))?;
                let source_start = match edit.media_time {
                    -1 => None,
                    ticks if ticks >= 0 => Some(source_sample(ticks as u64)?),
                    _ => return Err(invalid("invalid audio media edit time")),
                };
                if let Some(start) = source_start {
                    start
                        .checked_add(end - sample_frames)
                        .ok_or_else(|| invalid("audio source edit overflow"))?;
                }
                segments.push(Segment {
                    presentation: sample_frames..end,
                    source_start,
                });
                sample_frames = end;
            }
        }
        Ok(Self {
            segments,
            sample_frames,
        })
    }
    /// Seek in presentation samples; repeated segments select their own source
    /// range. None at EOF; a None source position inside a segment means silence.
    pub fn locate(&self, position: u64) -> Option<(usize, Option<u64>)> {
        let index = self
            .segments
            .partition_point(|s| s.presentation.end <= position);
        let segment = self.segments.get(index)?;
        Some((
            index,
            segment
                .source_start
                .map(|start| start + position - segment.presentation.start),
        ))
    }
}
