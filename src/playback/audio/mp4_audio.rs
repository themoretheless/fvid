//! Audio packet extraction from MP4 containers.
//!
//! The audio reader extracts packets from an AAC or uncompressed PCM track and
//! provides them to a decoder. Packet timestamps remain in the track media
//! timeline.

use crate::container::mp4::{Limits, Mp4Reader, Track};
use crate::{Result, invalid, unsupported};
use std::io::{Read, Seek};

/// Audio packet with presentation timestamp.
pub struct AudioPacket {
    /// Encoded audio data (AAC frame, etc.).
    pub data: Vec<u8>,
    /// Presentation timestamp in track timescale units.
    pub pts: i64,
    /// Duration in track timescale units.
    pub duration: i64,
    /// Sample index within the track.
    pub sample_index: usize,
}

/// MP4 audio track reader. Extracts encoded packets for decoding.
pub struct Mp4AudioReader<R> {
    demuxer: Mp4Reader<R>,
    track_index: usize,
    sample_index: usize,
    packet: Vec<u8>,
}

/// Audio fourccs decoded by the player, including the owned PCM endian mapping.
const CODECS: [[u8; 4]; 15] = [
    *b"mp4a",
    *b"raw ",
    *b"in24",
    *b"in32",
    *b"sowt",
    *b"twos",
    *b"fl32",
    *b"fl64",
    *b"ima4",
    *b"ms\x00\x02",
    *b"ms\x00\x11",
    *b"alac",
    *b"MAC3",
    *b"MAC6",
    *b"ac-3",
];

/// The decoder dispatch's name for a tag that cannot spell it itself. Not just the
/// ADPCM trio needs this: `ms\0\xNN` is not text at all, and both the player and
/// the coverage gate know these codings by what they code, not by four bytes. MACE's
/// two are spelled `MAC3` and `MAC6` in their entries: text, but not the names the
/// dispatch matches on.
fn codec_name(codec: &[u8; 4]) -> Option<&'static str> {
    Some(match codec {
        b"ima4" => "adpcm_ima_qt",
        b"ms\x00\x02" => "adpcm_ms",
        b"ms\x00\x11" => "adpcm_ima_wav",
        b"MAC3" => "mace3",
        b"MAC6" => "mace6",
        _ => return None,
    })
}

impl<R: Read + Seek> Mp4AudioReader<R> {
    /// Open the first audio track that a decoder exists for.
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        Self::open_at(reader, limits, 0)
    }

    /// Open the `nth` audio track a decoder exists for, counting in container
    /// order. Which one the player wants is the player's choice; the container
    /// only reports the ones a decoder exists for.
    pub fn open_at(reader: R, limits: Limits, nth: usize) -> Result<Self> {
        let demuxer = Mp4Reader::open(reader, limits)?;
        let index = match Self::supported(&demuxer).get(nth).copied() {
            Some(index) => index,
            // An audio track the container listed says what it is coded as, so a
            // refusal that names one is the player having no arm for that coding -
            // not a file it failed to read.
            None => match Self::first_audio(&demuxer) {
                Some(codec) => {
                    return Err(unsupported(&format!(
                        "MP4 audio track coded `{}` has no decoder here",
                        String::from_utf8_lossy(&codec)
                    )));
                }
                None => return Err(invalid("MP4 has no such audio track")),
            },
        };
        Self::from_demuxer(demuxer, index)
    }

    /// Fourcc of the first audio track the container found, whether or not a
    /// decoder exists for it: one this reader indexed, or one it read to the end
    /// of the sample entry and set aside for want of an arm.
    fn first_audio(demuxer: &Mp4Reader<R>) -> Option<[u8; 4]> {
        demuxer
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .map(|t| t.codec)
            .or_else(|| {
                demuxer
                    .refused()
                    .iter()
                    .find(|t| t.handler == *b"soun")
                    .map(|t| t.codec)
            })
    }

    /// Container indices of the audio tracks a decoder exists for.
    fn supported(demuxer: &Mp4Reader<R>) -> Vec<usize> {
        demuxer
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler == *b"soun" && CODECS.contains(&t.codec))
            .map(|(index, _)| index)
            .collect()
    }

    /// Open a specific audio track by index.
    pub fn from_demuxer(demuxer: Mp4Reader<R>, index: usize) -> Result<Self> {
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        if track.handler != *b"soun" {
            return Err(invalid("selected MP4 track is not audio"));
        }
        Ok(Self {
            demuxer,
            track_index: index,
            sample_index: 0,
            packet: Vec::new(),
        })
    }

    /// Read the next audio packet. Returns None at end of track.
    pub fn read_packet(&mut self) -> Result<Option<AudioPacket>> {
        let track_index = self.track_index;
        let sample_index = self.sample_index;
        let track = self.track();
        let sample = match track.samples.get(sample_index) {
            Some(s) => s,
            None => return Ok(None),
        };
        let pts = sample.pts;
        let duration = sample.duration as i64;

        self.demuxer
            .read_packet(track_index, sample_index, &mut self.packet)?;

        let packet = AudioPacket {
            data: self.packet.clone(),
            pts,
            duration,
            sample_index,
        };

        self.sample_index += 1;
        Ok(Some(packet))
    }

    /// The audio track metadata.
    pub fn track(&self) -> &Track {
        &self.demuxer.tracks()[self.track_index]
    }

    /// Restart from the first sample.
    pub fn rewind(&mut self) {
        self.demuxer.invalidate_position();
        self.sample_index = 0;
        self.packet.clear();
    }

    /// Seek to the sample with the greatest PTS at or before `pts`.
    pub fn seek(&mut self, pts: i64) -> i64 {
        let track = self.track();
        let index = track.samples.at_or_before(pts);
        let result_pts = track.samples.get(index).map_or(0, |s| s.pts);
        self.rewind();
        self.sample_index = index;
        result_pts
    }
}

/// Packets are timestamped in the *track* timescale, which differs from the
/// movie timescale in most files (sample rate versus a nominal 1000 Hz).
impl<R: Read + Seek + Send> crate::audio::AudioStream for Mp4AudioReader<R> {
    fn codec(&self) -> &str {
        let track = self.track();
        match codec_name(&track.codec) {
            Some(name) => name,
            None => std::str::from_utf8(&track.codec).unwrap_or(""),
        }
    }

    fn timescale(&self) -> u32 {
        self.track().timescale
    }

    fn sample_rate(&self) -> u32 {
        self.track().sample_rate
    }

    fn channels(&self) -> u16 {
        self.track().channels
    }

    fn bits_per_sample(&self) -> u16 {
        self.track().bit_depth
    }

    fn duration(&self) -> Option<std::time::Duration> {
        let track = self.track();
        if track.codec == *b"mp4a" && track.edits.len() == 1 && track.edits[0].media_time >= 0 && self.demuxer.movie_timescale() != 0 {
            let nanos = u128::from(track.edits[0].duration)*1_000_000_000/u128::from(self.demuxer.movie_timescale());
            return Some(std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)));
        }
        (track.timescale > 0 && track.duration > 0).then(|| {
            let nanos = u128::from(track.duration) * 1_000_000_000 / u128::from(track.timescale);
            std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
        })
    }

    fn time_of(&self, pts: i64) -> std::time::Duration {
        let track = self.track();
        let shift = if track.codec == *b"mp4a" && track.edits.len() == 1 && track.edits[0].media_time >= 0 { track.edits[0].media_time } else { 0 };
        if track.timescale == 0 { return std::time::Duration::ZERO; }
        std::time::Duration::from_secs_f64(pts.saturating_sub(shift).max(0) as f64/f64::from(track.timescale))
    }

    fn packet_sample_limit(&self, duration: u64) -> Result<Option<usize>> {
        if self.track().codec != *b"mp4a" { return Ok(None); }
        let rate = u128::from(self.track().sample_rate);
        let scale = u128::from(self.track().timescale);
        let samples = u128::from(duration) * rate;
        if scale == 0 || samples == 0 || samples % scale != 0 {
            return Err(invalid("MP4 AAC packet duration is not sample aligned"));
        }
        Ok(Some(usize::try_from(samples / scale).map_err(|_| invalid("MP4 AAC sample window overflow"))?))
    }

    fn present_decoded(&self, mut packet: crate::audio::AudioPacket, source_pts: i64) -> Result<Option<crate::audio::AudioPacket>> {
        let track = self.track();
        // Single media edits cover encoder priming and padding. Complex edit
        // sequences require replay/silence scheduling beyond this packet hook.
        if track.codec != *b"mp4a" || track.edits.len() != 1 || track.edits[0].media_time < 0 {
            return Ok(Some(packet));
        }
        let edit = &track.edits[0];
        let rate = u128::from(track.sample_rate);
        let scale = u128::from(track.timescale);
        let movie_scale = u128::from(self.demuxer.movie_timescale());
        if scale == 0 || movie_scale == 0 { return Err(invalid("zero MP4 audio presentation clock")); }
        let media_start = (edit.media_time as u128 * rate).div_ceil(scale);
        let length = (u128::from(edit.duration) * rate).div_ceil(movie_scale);
        let media_end = media_start.checked_add(length).ok_or_else(|| invalid("MP4 audio edit overflow"))?;
        let source_start = i128::from(source_pts) * i128::from(track.sample_rate) / i128::from(track.timescale);
        let stride = usize::from(track.channels).checked_mul(4).filter(|n| *n != 0).ok_or_else(|| invalid("invalid AAC PCM stride"))?;
        if !packet.data.len().is_multiple_of(stride) { return Err(invalid("unaligned AAC PCM buffer")); }
        let frames = packet.data.len()/stride;
        let first = (i128::try_from(media_start).map_err(|_| invalid("audio edit clock overflow"))?-source_start).clamp(0,frames as i128) as usize;
        let last = (i128::try_from(media_end).map_err(|_| invalid("audio edit clock overflow"))?-source_start).clamp(0,frames as i128) as usize;
        if first >= last { return Ok(None); }
        packet.data.copy_within(first*stride..last*stride,0);
        packet.data.truncate((last-first)*stride);
        packet.pts = u64::try_from(source_start+first as i128-media_start as i128).map_err(|_| invalid("negative audio presentation time"))?;
        packet.timebase_num = 1;
        packet.timebase_den = track.sample_rate;
        Ok(Some(packet))
    }

    fn extra_data(&self) -> &[u8] {
        &self.track().configuration
    }

    fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
        Mp4AudioReader::supported(&self.demuxer)
            .into_iter()
            .map(|index| {
                let track = &self.demuxer.tracks()[index];
                crate::audio::AudioTrack {
                    sample_rate: track.sample_rate,
                    channels: track.channels,
                    name: track.name.clone(),
                    language: track.language.clone(),
                }
            })
            .collect()
    }

    fn next_packet(&mut self) -> Result<Option<crate::audio::EncodedPacket>> {
        Ok(
            Mp4AudioReader::read_packet(self)?.map(|p| crate::audio::EncodedPacket {
                data: p.data,
                pts: p.pts,
                duration: p.duration,
            }),
        )
    }

    fn rewind(&mut self) {
        Mp4AudioReader::rewind(self);
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let track = self.track();
        let shift = if track.codec == *b"mp4a" && track.edits.len() == 1 && track.edits[0].media_time >= 0 { track.edits[0].media_time } else { 0 };
        Mp4AudioReader::seek(self, pts.saturating_add(shift)).saturating_sub(shift).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::Mp4AudioReader;
    use crate::audio::{AudioStream, AudioTrack};
    use crate::container::mp4::{Limits, Mp4Reader, SampleIndex};
    use std::io::Cursor;
    use std::mem::size_of;

    /// Two mono AAC tracks at different rates behind one H.264 track, written by
    /// ffmpeg so the listing comes from a real `trak` table rather than one
    /// assembled here:
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=2 \
    ///   -f lavfi -i sine=frequency=440:sample_rate=48000:duration=2 \
    ///   -f lavfi -i sine=frequency=880:sample_rate=32000:duration=2 \
    ///   -map 0:v -map 1:a -map 2:a -c:v libx264 -profile:v baseline \
    ///   -pix_fmt yuv420p -c:a aac -movflags +faststart two-audio.mp4
    /// ```
    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/two-audio.mp4");

    /// Every audio track a decoder exists for is listed in container order, and
    /// `open_at` takes a place in that list rather than a container index — the
    /// video track in between is skipped without the caller noticing.
    #[test]
    fn both_audio_tracks_are_listed_and_openable() {
        let first = Mp4AudioReader::open(Cursor::new(FIXTURE), Limits::default()).expect("audio");
        assert_eq!(
            first.audio_tracks(),
            vec![
                AudioTrack {
                    sample_rate: 48_000,
                    channels: 1,
                    name: String::new(),
                    language: String::new(),
                },
                AudioTrack {
                    sample_rate: 32_000,
                    channels: 1,
                    name: String::new(),
                    language: String::new(),
                },
            ]
        );
        assert_eq!((first.sample_rate(), first.channels()), (48_000, 1));

        let second =
            Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).expect("second");
        assert_eq!((second.sample_rate(), second.channels()), (32_000, 1));
        assert_eq!(second.track().codec, *b"mp4a");
        // Past the end is an error, not a silent return to the first track.
        assert!(Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 2).is_err());
    }

    /// The second track is not just listed: its own setup record decodes, so a
    /// track switch does not hand the old track's configuration to the decoder.
    #[test]
    fn the_second_track_decodes_at_its_own_rate() {
        let mut stream =
            Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).expect("second");
        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("AAC decoder");
        let mut frames = 0usize;
        while let Some(packet) = stream.next_packet().expect("packet") {
            let Some(pcm) = decoder
                .decode_encoded(
                    &packet.data,
                    packet.pts.max(0) as u64,
                    packet.duration.max(0) as u64,
                )
                .expect("decode")
            else {
                continue;
            };
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 32_000));
            frames += pcm.data.len() / (size_of::<f32>() * usize::from(stream.channels()));
        }
        // Two seconds at 32 kHz, allowing for the encoder's priming and the
        // trailing frame the container still lists.
        assert!(
            (60_000..66_000).contains(&frames),
            "decoded {frames} frames"
        );
    }

    /// The whole PCM path over a real file: a QuickTime track the container used
    /// to refuse, indexed, decoded and timed. Half a second of a 0.75-amplitude
    /// 440 Hz sine at 48 kHz stereo, muxed beside an AVC picture:
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=0.5 \
    ///   -f lavfi -i 'aevalsrc=0.75*sin(880*PI*t)|0.75*sin(880*PI*t):d=0.5:s=48000' \
    ///   -map 0:v -map 1:a -c:v libx264 -pix_fmt yuv420p -preset ultrafast \
    ///   -c:a pcm_s16le tests/fixtures/audio/pcm-screen.mov
    /// ```
    ///
    /// ffmpeg writes the lossless `sowt` tag, and its `mov` muxer puts one audio
    /// frame in every sample, so the index lists 24000 of them against the 24
    /// blocks `ffprobe` shows when it coalesces them on read. The frames are all
    /// the same four bytes long, so they are indexed as runs of chunks rather than
    /// one record each, which is what lets a recording of any length in at all.
    #[test]
    fn a_pcm_track_decodes_sample_by_sample() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let mut stream =
            Mp4AudioReader::open(Cursor::new(FIXTURE), Limits::default()).expect("PCM track");
        assert_eq!(stream.codec(), "sowt");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        // The sample entry's `sample_size`, which is what tells the decoder how
        // many bytes a sample has.
        assert_eq!(stream.bits_per_sample(), 16);
        assert!(stream.extra_data().is_empty(), "PCM has no setup record");

        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("PCM decoder");
        let bytes = usize::from(stream.channels()) * size_of::<f32>();
        let mut frames = 0usize;
        let mut packets = 0usize;
        let mut peak = 0.0f32;
        while let Some(packet) = stream.next_packet().expect("packet") {
            let pcm = decoder
                .decode_encoded(
                    &packet.data,
                    packet.pts.max(0) as u64,
                    packet.duration.max(0) as u64,
                )
                .expect("decode")
                .expect("PCM always yields a packet");
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 48_000));
            // A block's bytes widen by exactly the ratio of the two widths.
            assert_eq!(pcm.data.len(), packet.data.len() * 2);
            frames += pcm.data.len() / bytes;
            packets += 1;
            for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
            }
        }
        assert_eq!((packets, frames), (24_000, 24_000));
        // 24576 of 32768, the exact top of the sine as ffmpeg's own decode of the
        // file reports it: lossless coding makes the equality exact.
        assert_eq!(peak, 0.75);
    }

    /// Every PCM tag QuickTime writes, in one file: the two integer byte orders, a
    /// float track, and a 24-bit big-endian track that keeps its width and order
    /// in child atoms (`wave`/`frma`/`enda`) this reader does not resolve.
    ///
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.75*sin(880*PI*t)|0.75*sin(880*PI*t):d=0.05:s=48000' \
    ///   -map 0:a -map 0:a -map 0:a -map 0:a -c:a:0 pcm_s16le -c:a:1 pcm_s16be \
    ///   -c:a:2 pcm_f32le -c:a:3 pcm_s24be tests/fixtures/audio/pcm-tags.mov
    /// ```
    ///
    /// Each of the four tags this player reads has to reach the decoder as its
    /// own layout, because a byte order or width guessed wrongly still yields
    /// 2400 frames of plausible-looking noise. The same 0.75-amplitude sine in all
    /// of them is what says the samples came out at the right scale and sign — and
    /// `fl32` is written as a version 1 entry, so it also checks that the fields
    /// read before the extra ones still line up.
    #[test]
    fn each_pcm_tag_is_read_at_its_own_width_and_order() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-tags.mov");
        let listing = Mp4AudioReader::open(Cursor::new(FIXTURE), Limits::default()).expect("PCM");
        assert_eq!(listing.audio_tracks().len(), 4);
        assert!(Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 4).is_err());

        for (nth, codec, stated) in [
            (0, "sowt", 16u16),
            (1, "twos", 16),
            (2, "fl32", 16),
            (3, "in24", 24),
        ] {
            let mut stream = Mp4AudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), nth)
                .unwrap_or_else(|error| panic!("track {nth}: {error}"));
            assert_eq!(stream.codec(), codec);
            assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
            // What the sample entry's `sample_size` states. `fl32` states 16 while
            // carrying 32-bit floats, which is why a float's width comes from the
            // fourcc and not from this field.
            assert_eq!(stream.bits_per_sample(), stated, "{codec} sample entry");
            let mut decoder = crate::codec::make_audio_decoder(
                stream.codec(),
                stream.extra_data(),
                stream.sample_rate(),
                stream.channels(),
                stream.bits_per_sample(),
            )
            .expect("PCM decoder");
            let mut frames = 0usize;
            let mut peak = 0.0f32;
            while let Some(packet) = stream.next_packet().expect("packet") {
                let pcm = decoder
                    .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                    .expect("decode")
                    .expect("PCM always yields a packet");
                frames += pcm.data.len() / (2 * size_of::<f32>());
                for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                    peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
                }
            }
            assert_eq!(frames, 2_400, "{codec}");
            assert_eq!(peak, 0.75, "{codec}");
        }
    }

    /// The total a listener with no picture is shown. Each track states its own
    /// duration in its own timescale, so the two tracks of the fixture count the
    /// same sound in 48000 and 32000 ticks; the half second of PCM counts in
    /// samples. AAC single-edit duration excludes encoder priming/padding and
    /// reports the two audible seconds in the presentation timeline.
    #[test]
    fn each_track_states_its_own_length() {
        const TWO: &[u8] = include_bytes!("../../../tests/fixtures/audio/two-audio.mp4");
        for (nth, total) in [(0, 2_000_000_000_u64), (1, 2_000_000_000)] {
            let stream = Mp4AudioReader::open_at(Cursor::new(TWO), Limits::default(), nth).unwrap();
            assert_eq!(
                stream.duration(),
                Some(std::time::Duration::from_nanos(total)),
                "track {nth}"
            );
        }
        const PCM: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let stream = Mp4AudioReader::open(Cursor::new(PCM), Limits::default()).unwrap();
        assert_eq!(
            stream.duration(),
            Some(std::time::Duration::from_millis(500))
        );
    }

    /// The frames of an uncompressed track all state the same width and the same
    /// length, so its index keeps one record per chunk instead of one per frame.
    /// The half second of the fixture is 24 000 frames over 13 chunks, which a
    /// budget of 100 records covers; the same limits used to drop the track and
    /// leave only the picture beside it. What bounded the length of PCM a file
    /// could carry was the index, not the sound.
    ///
    /// Reading the whole track through that budget is what says the compact
    /// records point at the same bytes the per-frame ones did: a stride wrong by
    /// two still yields four bytes of plausible sine wave.
    #[test]
    fn a_pcm_track_indexes_as_runs_of_chunks() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let tight = Limits {
            samples: 100,
            ..Limits::default()
        };
        let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), tight).expect("both tracks index");
        let audio = mp4
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .expect("PCM track");
        let SampleIndex::Uniform(runs) = &audio.samples else {
            panic!("expected runs of chunks, got {:?}", audio.samples);
        };
        assert_eq!(
            (
                runs.count,
                runs.runs.len(),
                runs.bytes_per_frame,
                runs.duration
            ),
            (24_000, 13, 4, 1)
        );
        // Inside a run the frames are the next four bytes and the next tick. The
        // runs themselves are not contiguous: the writer interleaves them with
        // the picture's chunks, which is the only reason there are 13 of them.
        for frame in 0..runs.count {
            let at = audio.samples.get(frame).unwrap();
            assert_eq!(
                (at.pts, at.dts, at.duration, at.size, at.sync),
                (frame as i64, frame as u64, 1, 4, true)
            );
        }
        let mut covered = 0usize;
        for run in &runs.runs {
            assert_eq!(run.first, covered, "runs follow one another");
            for frame in run.first..run.first + run.frames {
                let at = audio.samples.get(frame).unwrap();
                assert_eq!(at.offset, run.offset + 4 * (frame - run.first) as u64);
            }
            covered += run.frames;
        }
        assert_eq!(covered, runs.count);
        assert!(audio.samples.get(runs.count).is_none(), "no frame past end");

        let mut stream = Mp4AudioReader::open(Cursor::new(FIXTURE), tight).expect("PCM");
        let mut frames = 0usize;
        while let Some(packet) = stream.next_packet().expect("packet") {
            assert_eq!((packet.pts, packet.duration), (frames as i64, 1));
            frames += 1;
        }
        assert_eq!(frames, 24_000);
        // A seek lands on the frame named, in a track it never indexed per frame.
        assert_eq!(stream.seek(12_345), 12_345);
        let packet = stream.next_packet().expect("packet").expect("frame");
        assert_eq!(packet.pts, 12_345);
    }

    /// The budget still means something: a track whose chunks alone outnumber it
    /// is left out the way an over-long one used to be, because the picture
    /// beside it is worth more than either. One record short of what the two
    /// tracks of the fixture together ask for is the smallest case that reaches
    /// the rule, and it only works because the picture is indexed first.
    #[test]
    fn a_pcm_track_over_the_sample_budget_leaves_the_picture() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-screen.mov");
        let wide = Mp4Reader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
        assert_eq!(wide.tracks()[0].handler, *b"vide", "picture indexed first");
        let cost = |handler: [u8; 4]| {
            wide.tracks()
                .iter()
                .filter(|t| t.handler == handler)
                .map(|t| t.samples.cost())
                .sum::<usize>()
        };
        let tight = Limits {
            samples: cost(*b"vide") + cost(*b"soun") - 1,
            ..Limits::default()
        };
        drop(wide);
        let mp4 = Mp4Reader::open(Cursor::new(FIXTURE), tight).expect("the file still opens");
        assert_eq!(mp4.tracks().len(), 1);
        assert_eq!(mp4.tracks()[0].handler, *b"vide");
        assert!(Mp4AudioReader::open(Cursor::new(FIXTURE), tight).is_err());
    }

    /// The three audio tracks of the named file, listed the way the player
    /// shows them: the title where the file gives one, the language where it
    /// only states a language, and the layout the track carries after either.
    /// `tests/fixtures/tracks/named.mp4`, whose command `tests/mp4.rs` records.
    #[test]
    fn a_named_file_lists_its_audio_tracks_by_what_it_says() {
        const NAMED: &[u8] = include_bytes!("../../../tests/fixtures/tracks/named.mp4");
        let stream = Mp4AudioReader::open(Cursor::new(NAMED), Limits::default()).expect("audio");
        let labels: Vec<String> = stream
            .audio_tracks()
            .iter()
            .map(crate::audio::AudioTrack::label)
            .collect();
        assert_eq!(
            labels,
            [
                "Первая · 1 ch 48000 Hz",
                "fre · 1 ch 32000 Hz",
                "1 ch 44100 Hz",
            ]
        );
    }
}
