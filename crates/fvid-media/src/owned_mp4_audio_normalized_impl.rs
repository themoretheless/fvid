// Shared f32/f64 presentation normalization. Wrappers supply decoder/DSP aliases.
use mp4_export_clock::{Time as Mp4ExportTime, Timeline as Mp4ExportTimeline};
struct Mp4NormalizationRun {
    filter: Mp4OutputResampler,
    matrix: Vec<Vec<f64>>,
    next_output: i128,
    input_channels: usize,
    output_channels: usize,
    chunk_frames: usize,
}
impl Mp4NormalizationRun {
    fn new(
        origin: Mp4ExportTime,
        input_rate: u32,
        output_rate: u32,
        input_channels: u16,
        output_channels: u16,
        matrix: Vec<Vec<f64>>,
    ) -> Result<Self> {
        let (whole, phase) = origin.output_origin(output_rate)?;
        let filter = Mp4OutputResampler::new_with_origin(
            Vec::new(),
            input_rate,
            output_rate,
            output_channels,
            0,
            phase,
        )?;
        let next_output = whole
            .checked_add(i128::from(filter.output_position()))
            .ok_or_else(|| invalid("MP4 normalization output clock overflow"))?;
        let chunk_frames =
            (4096u64 * u64::from(input_rate) / u64::from(output_rate)).clamp(1, 256) as usize;
        Ok(Self {
            filter,
            matrix,
            next_output,
            input_channels: usize::from(input_channels),
            output_channels: usize::from(output_channels),
            chunk_frames,
        })
    }
    fn publish(
        &mut self,
        output: &mut impl std::io::Write,
        window: std::ops::Range<i128>,
        cursor: &mut i128,
    ) -> Result<()> {
        let bytes = self.filter.take_output();
        let stride = self.output_channels * Mp4TimelineDecoder::SAMPLE_BYTES;
        if !bytes.len().is_multiple_of(stride) {
            return Err(invalid("invalid normalized MP4 PCM stride"));
        }
        let end = self
            .next_output
            .checked_add((bytes.len() / stride) as i128)
            .ok_or_else(|| invalid("MP4 normalization output clock overflow"))?;
        let first = self.next_output.max(window.start).max(*cursor);
        let last = end.min(window.end);
        if first < last {
            if first != *cursor {
                return Err(invalid("MP4 audio edit extends outside available samples"));
            }
            let a = usize::try_from(first - self.next_output)
                .map_err(|_| invalid("MP4 normalization offset overflow"))?;
            let b = usize::try_from(last - self.next_output)
                .map_err(|_| invalid("MP4 normalization offset overflow"))?;
            output.write_all(&bytes[a * stride..b * stride])?;
            *cursor = last;
        }
        self.next_output = end;
        Ok(())
    }
    fn feed(
        &mut self,
        samples: &[Mp4PcmScalar],
        output: &mut impl std::io::Write,
        window: std::ops::Range<i128>,
        cursor: &mut i128,
        control: &mut DecodeProgress<'_>,
    ) -> Result<()> {
        use std::io::Write;
        if !samples.len().is_multiple_of(self.input_channels) {
            return Err(invalid("invalid MP4 normalization input stride"));
        }
        let mut mixed = Vec::new();
        mixed
            .try_reserve_exact(
                self.chunk_frames * self.output_channels * Mp4TimelineDecoder::SAMPLE_BYTES,
            )
            .map_err(|_| invalid("MP4 normalization allocation failed"))?;
        for chunk in samples.chunks(self.chunk_frames * self.input_channels) {
            control.check()?;
            mixed.clear();
            for frame in chunk.chunks_exact(self.input_channels) {
                for channel in 0..self.output_channels {
                    let value = frame
                        .iter()
                        .enumerate()
                        .map(|(i, &v)| (v as f64) * self.matrix[i][channel])
                        .sum::<f64>() as Mp4PcmScalar;
                    mixed.extend_from_slice(&value.to_le_bytes());
                }
            }
            self.filter.write_all(&mixed)?;
            self.publish(output, window.clone(), cursor)?;
        }
        Ok(())
    }
    fn finish(
        &mut self,
        output: &mut impl std::io::Write,
        window: std::ops::Range<i128>,
        cursor: &mut i128,
    ) -> Result<()> {
        self.filter.finish()?;
        self.publish(output, window, cursor)
    }
}

fn decode_mp4_audio_normalized<R: std::io::Read + std::io::Seek>(
    mut reader: Mp4TimelineReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    index: usize,
    track: Mp4NormalizationTrack,
    initial_description: u32,
    ps: Vec<bool>,
    decoder: Mp4TimelineDecoder,
    target: Option<(u32, u16, u32)>,
    control: &mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    let (rate, channels, destination) = target.unwrap_or_else(|| {
        (
            decoder.sample_rate(),
            decoder.channels(),
            decoder.normalization_mask(),
        )
    });
    if rate == 0 || channels == 0 || channels > 8 {
        return Err(invalid(
            "unsupported MP4 normalization destination geometry",
        ));
    }
    // Validate routing before caller-owned output, including configurations reached
    // within the requested prefix. A failed file export never publishes a spool.
    let active_samples = if track.edits.is_empty() {
        control
            .packet_limit()
            .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
            .unwrap_or(track.samples.len())
            .min(track.samples.len())
    } else {
        track.samples.len()
    };
    let mut opaque_identity=false;
    if destination==0 && target.is_none() {
        let first=track.decoder_setup(initial_description)?;
        if first.codec==*b"mp4a" {
            let positions=mp4_opaque_positions(&first.configuration).map_err(|e|invalid(&e))?;
            opaque_identity=positions.as_ref().is_some_and(|p|p.len()==usize::from(channels));
            for i in 0..track.sample_descriptions.len() {
                if !(0..active_samples).any(|n|track.samples.get(n).is_some_and(|s|s.description_index as usize==i+1)) {continue;}
                let setup=track.decoder_setup((i+1)as u32)?;
                if setup.codec!=*b"mp4a" || setup.channels!=channels || mp4_opaque_positions(&setup.configuration).map_err(|e|invalid(&e))?!=positions {opaque_identity=false;break;}
            }
        }
    }
    if destination == 0 && !opaque_identity {
        return Err(invalid(
            "MP4 normalization requires an explicit destination speaker layout",
        ));
    }
    let make_decoder = |id: u32| -> Result<Mp4TimelineDecoder> {
        let setup = track.decoder_setup(id)?;
        if ps[(id - 1) as usize] {
            Mp4TimelineDecoder::with_in_band_ps(&setup)
        } else {
            Mp4TimelineDecoder::new(&setup)
        }
    };
    let matrix_for = |id: u32, d: &Mp4TimelineDecoder| -> Result<Vec<Vec<f64>>> {
        let setup = track.decoder_setup(id)?;
        if setup.sample_rate != d.sample_rate()
            || (!ps[(id - 1) as usize] && setup.channels != d.channels())
        {
            return Err(invalid(
                "MP4 container and normalization decoder geometry disagree",
            ));
        }
        if opaque_identity {
            return Ok((0..usize::from(channels)).map(|i|(0..usize::from(channels)).map(|j|if i==j {1.}else{0.}).collect()).collect());
        }
        mp4_normalization_coefficients(
            setup.codec,
            &setup.configuration,
            d.sample_rate(),
            d.channels(),
            d.normalization_mask(),
            ps[(id - 1) as usize],
            channels,
            destination,
        )
        .map_err(|e| invalid(&e))
    };
    for i in 0..track.sample_descriptions.len() {
        control.check()?;
        if (0..active_samples).any(|n| {
            track
                .samples
                .get(n)
                .is_some_and(|s| s.description_index as usize == i + 1)
        }) {
            matrix_for((i + 1) as u32, &make_decoder((i + 1) as u32)?)?;
        }
    }
    let packet_clocks=mp4_packet_clock_runs(&mut reader,index,control)?;
    let mut timeline =
        Mp4ExportTimeline::new(&track.edits, track.timescale, reader.movie_timescale())?;
    if track.edits.is_empty() {
        let mut end = Mp4ExportTime::ticks(track.duration, track.timescale)?;
        if track.samples.get(0).is_some_and(|s|s.pts==0) {
            if let Some(last)=packet_clocks.last() {
                if last.last==track.samples.len() && last.quantized && mp4_tick_matches(last.end,i128::from(track.duration),track.timescale)? {end=last.end;}
            }
        }
        timeline.segments.push(mp4_export_clock::Segment {
            start: Mp4ExportTime::ZERO,
            end,
            media: Some(Mp4ExportTime::ZERO),
        });
        timeline.end = end;
    }
    let total = timeline.end.ceil_frames(rate)?;
    let boundary = |d: Duration| -> Result<i128> {
        Mp4ExportTime::new(
            i128::try_from(d.as_nanos()).map_err(|_| invalid("MP4 interval overflow"))?,
            1_000_000_000,
        )?
        .ceil_frames(rate)
    };
    let (from, to) = if let Some((a, b)) = interval {
        (boundary(a)?, boundary(b)?.min(total))
    } else {
        (0, total)
    };
    if from >= to {
        return Err(invalid("audio interval contains no samples"));
    }
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut opening_decoder = Some(decoder);
    let mut packet = Vec::new();
    for segment in timeline.segments {
        control.check()?;
        if control.packet_limit_reached() {
            break;
        }
        let begin = from.max(segment.start.ceil_frames(rate)?);
        let end = to.min(segment.end.ceil_frames(rate)?);
        if begin >= end {
            continue;
        }
        let Some(media) = segment.media else {
            let stride = usize::from(channels) * Mp4TimelineDecoder::SAMPLE_BYTES;
            let zeros = [0u8; 4096];
            let mut remaining = u64::try_from(end - begin)
                .ok()
                .and_then(|n| n.checked_mul(stride as u64))
                .ok_or_else(|| invalid("MP4 silence overflow"))?;
            while remaining > 0 {
                control.check()?;
                let n = remaining.min(zeros.len() as u64) as usize;
                output.write_all(&zeros[..n])?;
                remaining -= n as u64;
            }
            stats.sample_frames +=
                u64::try_from(end - begin).map_err(|_| invalid("MP4 output count overflow"))?;
            continue;
        };
        let window = begin..end;
        let mut cursor = begin;
        let mut decoder = if let Some(d) = opening_decoder.take() {
            d
        } else {
            make_decoder(initial_description)?
        };
        let mut active = initial_description;
        let mut native_cursor = 0u64;
        let mut pending = std::collections::VecDeque::new();
        let mut sample_index = 0usize;
        let mut clock_index=0usize;
        let mut expected: Option<Mp4ExportTime> = None;
        let mut run: Option<Mp4NormalizationRun> = None;
        loop {
            control.check()?;
            if cursor == end {
                break;
            }
            let terminal = sample_index == track.samples.len() || control.packet_limit_reached();
            let changing = !terminal && {
                let id = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing MP4 sample"))?
                    .description_index;
                track.sample_descriptions[(id - 1) as usize]
                    != track.sample_descriptions[(active - 1) as usize]
            };
            let (start, duration, samples) = if terminal || (changing && !pending.is_empty()) {
                let Some(samples) = decoder.finish_delayed()? else {
                    if !pending.is_empty() {
                        return Err(invalid("delayed MP4 audio left an undrained source window"));
                    }
                    if let Some(r) = &mut run {
                        r.finish(output, window.clone(), &mut cursor)?;
                    }
                    break;
                };
                let (start, duration) = pending
                    .pop_front()
                    .ok_or_else(|| invalid("delayed MP4 audio has no source window"))?;
                (start, duration, samples)
            } else {
                let sample = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing MP4 sample"))?;
                if changing {
                    if let Some(r) = &mut run {
                        r.finish(output, window.clone(), &mut cursor)?;
                    }
                    if cursor == end {
                        break;
                    }
                    run = None;
                    decoder = make_decoder(sample.description_index)?;
                    active = sample.description_index;
                    native_cursor = 0;
                }
                let declared_start = Mp4ExportTime::signed_ticks(sample.pts, track.timescale)?;
                let declared_duration = Mp4ExportTime::ticks(u64::from(sample.duration), track.timescale)?;
                if expected.is_some_and(|at| at != declared_start) {return Err(invalid("non-contiguous MP4 audio timeline"));}
                expected=Some(declared_start.add(declared_duration)?);
                reader.read_packet(index, sample_index, &mut packet)?;
                while packet_clocks.get(clock_index).is_some_and(|r|sample_index>=r.last) {clock_index+=1;}
                let clock=packet_clocks.get(clock_index).filter(|r|sample_index>=r.first).ok_or_else(||invalid("missing MP4 packet clock run"))?;
                let (start,duration,frames)=if clock.quantized {
                    let setup=track.description_for_sample(sample_index).ok_or_else(||invalid("missing MP4 clock description"))?;
                    let frames=mp4_coded_packet_frames(&setup.codec,&setup.configuration,decoder.sample_rate(),setup.channels,setup.bit_depth,packet.len(),Some(&packet)).map_err(|e|invalid(&e))?;
                    (clock.origin.add(Mp4ExportTime::new(i128::from(native_cursor),u128::from(decoder.sample_rate()))?)?,Mp4ExportTime::new(i128::from(frames),u128::from(decoder.sample_rate()))?,frames)
                } else {
                    let frames=declared_duration.ceil_frames(decoder.sample_rate())?;
                    if frames<=0 || Mp4ExportTime::new(frames,u128::from(decoder.sample_rate()))?!=declared_duration {return Err(invalid("MP4 audio packet clock is not sample aligned"));}
                    (declared_start,declared_duration,u64::try_from(frames).map_err(|_|invalid("MP4 native frame count overflow"))?)
                };
                let samples = decoder.decode_timed(&packet, native_cursor, frames)?;
                native_cursor = native_cursor
                    .checked_add(frames)
                    .ok_or_else(|| invalid("MP4 native clock overflow"))?;
                control.packet(packet.len())?;
                sample_index += 1;
                if decoder.delayed() {
                    pending.push_back((start, duration));
                    if pending.len() > 3 {
                        return Err(invalid("delayed MP4 source lookahead exceeded"));
                    }
                    let Some(samples) = samples else {
                        continue;
                    };
                    let (start, duration) = pending
                        .pop_front()
                        .ok_or_else(|| invalid("delayed MP4 source identity mismatch"))?;
                    (start, duration, samples)
                } else {
                    (
                        start,
                        duration,
                        samples.ok_or_else(|| invalid("immediate MP4 audio returned no PCM"))?,
                    )
                }
            };
            let input_channels = usize::from(decoder.channels());
            let frames = usize::try_from(duration.ceil_frames(decoder.sample_rate())?)
                .map_err(|_| invalid("MP4 native duration overflow"))?;
            if !samples.len().is_multiple_of(input_channels)
                || frames == 0
                || frames > samples.len() / input_channels
            {
                return Err(invalid(
                    "MP4 audio packet duration disagrees with decoded samples",
                ));
            }
            if run.is_none() {
                let origin = segment.start.add(start.sub(media)?)?;
                run = Some(Mp4NormalizationRun::new(
                    origin,
                    decoder.sample_rate(),
                    rate,
                    decoder.channels(),
                    channels,
                    matrix_for(active, &decoder)?,
                )?);
            }
            run.as_mut().unwrap().feed(
                &samples[..frames * input_channels],
                output,
                window.clone(),
                &mut cursor,
                control,
            )?;
            stats.decoded_frames += 1;
        }
        if cursor != end && !control.packet_limit_reached() {
            return Err(invalid("MP4 audio edit extends outside available samples"));
        }
        stats.sample_frames = stats
            .sample_frames
            .checked_add(
                u64::try_from(cursor - begin).map_err(|_| invalid("MP4 output count overflow"))?,
            )
            .ok_or_else(|| invalid("MP4 output count overflow"))?;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("MP4 audio edit contains no samples"));
    }
    Ok(stats)
}

include!("owned_mp4_audio_clock_runs_impl.rs");
