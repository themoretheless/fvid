fn check_cancel(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
fn sample_ns(samples: u64, rate: u32) -> Result<u64> {
    u64::try_from((u128::from(samples) * 1_000_000_000 + u128::from(rate) / 2) / u128::from(rate))
        .map_err(|_| invalid("AAC nanosecond timestamp overflow"))
}
pub(crate) struct AacPacketPlan {
    pub count: usize,
    pub delay: u64,
    padding: i64,
    frame: u64,
    pub rate: u32,
}
impl AacPacketPlan {
    pub fn packet(&self, index: usize) -> Result<(u64, u64, i64)> {
        let begin = sample_ns(index as u64 * self.frame, self.rate)?;
        let end = sample_ns((index as u64 + 1) * self.frame, self.rate)?;
        Ok((
            begin,
            end - begin,
            if index + 1 == self.count {
                self.padding
            } else {
                0
            },
        ))
    }
}
pub(crate) fn aac_packet_plan(
    track: &Mp4AacTrack,
    movie_scale: u32,
    cancel: Option<&CancelFlag>,
) -> Result<AacPacketPlan> {
    aac_packet_plan_window(track, movie_scale, None, cancel)
}
pub(crate) fn aac_packet_plan_window(
    track: &Mp4AacTrack,
    movie_scale: u32,
    interval: Option<(i64, i64)>,
    cancel: Option<&CancelFlag>,
) -> Result<AacPacketPlan> {
    if track.handler != *b"soun" || track.codec != *b"mp4a" {
        return Err(invalid("MP4 track is not AAC"));
    }
    let asc = aac_specific_config(&track.configuration)?;
    let config = AacConfig::parse(asc)?;
    if track.timescale == 0
        || track.sample_rate != config.sample_rate
        || track.channels != u16::from(config.channels)
    {
        return Err(invalid("MP4 AAC clock or geometry mismatch"));
    }
    let rate = u128::from(config.sample_rate);
    let position = |ticks: u64| -> Result<u64> {
        let n = u128::from(ticks) * rate;
        let d = u128::from(track.timescale);
        if n % d != 0 {
            return Err(invalid("AAC time is not sample aligned"));
        }
        u64::try_from(n / d).map_err(|_| invalid("AAC sample position overflow"))
    };
    let media_end = position(track.duration)?;
    let (mut start, mut end) = match track.edits.as_slice() {
        [] => (0, media_end),
        [edit] if edit.media_time >= 0 && movie_scale != 0 => {
            let start = position(edit.media_time as u64)?;
            let length =
                u64::try_from((u128::from(edit.duration) * rate).div_ceil(u128::from(movie_scale)))
                    .map_err(|_| invalid("AAC edit duration overflow"))?;
            (
                start,
                start
                    .checked_add(length)
                    .ok_or_else(|| invalid("AAC edit endpoint overflow"))?,
            )
        }
        _ => {
            return Err(invalid(
                "AAC packet remux requires one contiguous media edit",
            ));
        }
    };
    if start >= end || end > media_end {
        return Err(invalid("AAC edit exceeds media samples"));
    }
    if let Some((from, to)) = interval {
        if from < 0 || from >= to {
            return Err(invalid("audio interval requires 0 <= from < to"));
        }
        let samples = |us: i64| -> Result<u64> {
            u64::try_from((us as u128 * rate).div_ceil(1_000_000))
                .map_err(|_| invalid("AAC interval sample overflow"))
        };
        let origin = start;
        start = origin.checked_add(samples(from)?)
            .ok_or_else(|| invalid("AAC interval endpoint overflow"))?;
        end = end.min(origin.checked_add(samples(to)?)
            .ok_or_else(|| invalid("AAC interval endpoint overflow"))?);
        if start >= end {
            return Err(invalid("audio interval contains no samples"));
        }
    }
    let frame = u64::from(config.frame_samples);
    let count =
        usize::try_from(end.div_ceil(frame)).map_err(|_| invalid("AAC packet count overflow"))?;
    if count > track.samples.len() {
        return Err(invalid("AAC edit exceeds packet index"));
    }
    // Validate before producing a header. The AAC frame clock, rather than a
    // shortened final stts duration, determines the actual decoded frame size.
    for i in 0..track.samples.len() {
        check_cancel(cancel)?;
        let sample = track
            .samples
            .get(i)
            .ok_or_else(|| invalid("missing AAC sample"))?;
        let expected = (i as u64)
            .checked_mul(frame)
            .ok_or_else(|| invalid("AAC timeline overflow"))?;
        let duration = position(u64::from(sample.duration))?;
        if sample.pts < 0
            || sample.pts as u64 != sample.dts
            || position(sample.dts)? != expected
            || duration == 0
            || duration > frame
            || (i + 1 != track.samples.len() && duration != frame)
        {
            return Err(invalid(
                "AAC packet remux requires a contiguous frame clock",
            ));
        }
        if i + 1 == track.samples.len() && expected.checked_add(duration) != Some(media_end) {
            return Err(invalid("AAC media duration disagrees with packet timeline"));
        }
    }
    let delay = sample_ns(start, config.sample_rate)?;
    let coded_end = (count as u64)
        .checked_mul(frame)
        .ok_or_else(|| invalid("AAC timeline overflow"))?;
    let padding = i64::try_from(sample_ns(coded_end - end, config.sample_rate)?)
        .map_err(|_| invalid("AAC padding overflow"))?;
    Ok(AacPacketPlan {
        count,
        delay,
        padding,
        frame,
        rate: config.sample_rate,
    })
}
