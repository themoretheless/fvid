//! Native WebM/Matroska AVC, HEVC, VP9, AV1 and FFV1 playback with bounded decode-ahead and source timestamps.
use crate::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture, hevc_decoder::HevcDecoder};
use crate::{
    Result,
    codec::{
        vp9,
        vp9_decoder::{Decoded, Decoder},
    },
    color::hdr::{ColourDescription, HdrMetadata},
    container::webm::{Limits, WebmReader},
    invalid,
    playback_native::{AvcColour, PackedPlanar, Planar8},
};
use std::sync::Arc;
use std::{
    io::{Read, Seek},
    time::Duration,
};

enum Picture {
    Coded(Arc<IntraPicture>, AvcColour),
    Ffv1(Arc<PackedPlanar>),
    Vp9(Box<Decoded>),
    Av1(crate::codec::av1_decoder::Decoded),
}
enum VideoDecoder {
    Avc(Box<AvcDecoder>),
    Hevc(Box<HevcDecoder>),
    Ffv1(Box<crate::codec::ffv1_decoder::Decoder>),
    Vp9(Box<Decoder>),
    Av1(Box<crate::codec::av1_decoder::Decoder>),
}
struct Frame {
    decoded: Picture,
    pts: i64,
    duration: Option<u64>,
}
/// A decoded visible frame plus its display colour metadata, ready for either
/// CPU RGB conversion or packing into 8-bit planes for the GPU.
struct CurrentFrame {
    decoded: Picture,
    size: [u32; 2],
    depth: u8,
    full_range: bool,
    color_space: u8,
    monochrome: bool,
}
pub struct WebmVideoReader<R> {
    demux: WebmReader<R>,
    decoder: VideoDecoder,
    track: u64,
    index: usize,
    pending: Option<Frame>,
    future_pts: Option<Vec<i64>>,
    queued: Vec<Frame>,
    queue_budget: usize,
    rgb: Vec<u8>,
    dimensions: [usize; 2],
    pixel_aspect: (u32, u32),
    rotation: u16,
    /// The stated crop borders as pixel insets into the coded frame, `[0; 4]`
    /// when the track states none it can keep.
    insets: [u32; 4],
    /// The signal the track's own `Colour` element states, zeroes where it wrote
    /// none.
    colour: ColourDescription,
    /// The light the track's `MasteringMetadata` and `MaxCLL`/`MaxFALL` name.
    hdr: HdrMetadata,
    /// What an AV1 track states about its own pictures in the bytes a reader
    /// holds the moment the file is open: the signal of its sequence header and
    /// the light of its metadata OBUs, read from the track's CodecPrivate and
    /// from its first packet, which is where an encoder that writes its light
    /// in-band writes it. A caller grading the first picture asks then. `None`
    /// for VP9, which states no signal of its own.
    open_signal: Option<(ColourDescription, HdrMetadata)>,
    /// Which codec the track's CodecID picked, named for a reader
    /// rather than for a match arm.
    codec: &'static str,
    start: u128,
    end: u128,
    base: Option<i64>,
    last_duration: u64,
    default_duration: Option<u64>,
    rgb_budget: usize,
    failed: bool,
    frames: u64,
}
impl<R: Read + Seek> WebmVideoReader<R> {
    pub fn open(reader: R, budget: usize) -> Result<Self> {
        let mut demux = WebmReader::open(reader, Limits::default())?;
        let track = demux
            .tracks
            .iter()
            .find(|t| {
                t.kind == 1
                    && matches!(
                        t.codec.as_str(),
                        "V_VP9" | "V_AV1" | "V_MPEG4/ISO/AVC" | "V_MPEGH/ISO/HEVC" | "V_FFV1"
                    )
            })
            .ok_or_else(|| {
                invalid("WebM/Matroska has no supported AVC, HEVC, VP9, AV1 or FFV1 video track")
            })?;
        let mut default_duration =
            (track.default_duration_ns > 0).then_some(track.default_duration_ns);
        let ffv1 = track.codec == "V_FFV1";
        let ffv1_size = if ffv1 {
            if !track.codec_private.is_empty() {
                return Err(crate::unsupported(
                    "FFV1 configuration-record versions are not implemented",
                ));
            }
            Some((
                u32::try_from(track.width).map_err(|_| invalid("FFV1 width overflow"))? as usize,
                u32::try_from(track.height).map_err(|_| invalid("FFV1 height overflow"))? as usize,
            ))
        } else {
            None
        };
        let av1 = track.codec == "V_AV1";
        let avc = track.codec == "V_MPEG4/ISO/AVC";
        let hevc = track.codec == "V_MPEGH/ISO/HEVC";
        let pixel_aspect = track.pixel_aspect();
        let rotation = track.rotation;
        // The validated crop borders are each smaller than the coded size they
        // divide; one that cannot be a pixel offset on this machine is read as
        // a file stating a crop it cannot keep.
        let insets = {
            let mut sides = [0u32; 4];
            let mut keeps = true;
            for (side, value) in sides.iter_mut().zip(track.crop) {
                *side = u32::try_from(value).unwrap_or(0);
                keeps &= u32::try_from(value).is_ok();
            }
            if keeps { sides } else { [0; 4] }
        };
        // What the muxer wrote about the signal, kept beside the track it
        // describes. An AV1 stream states the same triple again in its own
        // sequence header, and a track with no `Colour` element is graded by
        // that instead, so the two are read apart.
        let (colour, hdr) = (track.colour, track.hdr);
        let (track, private) = (track.number, track.codec_private.clone());
        let rgb_budget = budget / 4;
        // An AV1 track's own statement, read before a single picture is
        // decoded: the configuration OBUs the muxer put in CodecPrivate and the
        // first packet of the track, which is where SVT-AV1 puts a mastering
        // volume and a content light level a Matroska `Colour` element names
        // neither of. Bytes that do not parse state nothing; the walk reports
        // what it makes of them when it meets them again.
        let open_signal = av1.then(|| {
            let mut seed = crate::codec::av1_metadata::signal_from_bytes(&private);
            if let Some(index) = demux.packets.iter().position(|p| p.track == track)
                && let Ok(packet) = demux.read_packet(index) {
                    // The packet is the later of the two statements, so it wins
                    // and the CodecPrivate fills whatever it left out.
                    let (stated, light) = crate::codec::av1_metadata::signal_from_bytes(&packet);
                    seed = (stated.filled_with(seed.0), light.filled_with(seed.1));
                }
            seed
        });
        let queue_budget = (budget - rgb_budget) / 2;
        let future_pts = if avc || hevc {
            // Reordered codecs need a suffix minimum of packet PTS to know when
            // a decoded picture can be emitted. Packet payloads remain lazy.
            demux.scan_all()?;
            let mut future = Vec::new();
            future
                .try_reserve_exact(demux.packets.len())
                .map_err(|_| invalid("cannot allocate Matroska reorder index"))?;
            let mut minimum = i64::MAX;
            for packet in demux.packets.iter().rev() {
                if packet.track == track && !packet.invisible {
                    minimum = minimum.min(packet.pts_ns);
                }
                future.push(minimum);
            }
            future.reverse();
            if default_duration.is_none() {
                let last = demux
                    .packets
                    .iter()
                    .filter(|p| p.track == track && !p.invisible)
                    .map(|p| p.pts_ns)
                    .max();
                if let Some(last) = last {
                    let previous = demux
                        .packets
                        .iter()
                        .filter(|p| p.track == track && !p.invisible && p.pts_ns < last)
                        .map(|p| p.pts_ns)
                        .max();
                    default_duration = previous.and_then(|previous| {
                        u64::try_from(i128::from(last) - i128::from(previous)).ok()
                    });
                }
            }
            Some(future)
        } else {
            None
        };
        Ok(Self {
            demux,
            decoder: if let Some((w, h)) = ffv1_size {
                VideoDecoder::Ffv1(Box::new(crate::codec::ffv1_decoder::Decoder::new(
                    w,
                    h,
                    queue_budget,
                )?))
            } else if avc {
                VideoDecoder::Avc(Box::new(AvcDecoder::new(&private, queue_budget)?))
            } else if hevc {
                VideoDecoder::Hevc(Box::new(HevcDecoder::from_configuration(&private, queue_budget)?))
            } else if av1 {
                VideoDecoder::Av1(Box::new(crate::codec::av1_decoder::Decoder::from_configuration(
                    &private, (budget - rgb_budget) / 12 * 10,
                )?))
            } else {
                VideoDecoder::Vp9(Box::new(Decoder::new((budget - rgb_budget) / 12 * 10)))
            },
            track,
            index: 0,
            pending: None,
            future_pts,
            queued: Vec::new(),
            queue_budget,
            rgb: Vec::new(),
            dimensions: [0; 2],
            pixel_aspect,
            rotation,
            insets,
            colour,
            hdr,
            codec: if ffv1 {
                "FFV1"
            } else if avc {
                "H.264"
            } else if hevc {
                "H.265"
            } else if av1 {
                "AV1"
            } else {
                "VP9"
            },
            open_signal,
            start: 0,
            end: 0,
            base: None,
            last_duration: 33_333_333,
            default_duration,
            rgb_budget,
            failed: false,
            frames: 0,
        })
    }
    pub fn dimensions(&self) -> [usize; 2] {
        self.dimensions
    }
    pub fn rotation(&self) -> u16 {
        self.rotation
    }
    pub fn pixel_aspect(&self) -> (u32, u32) {
        if matches!(self.rotation, 90 | 270) {
            (self.pixel_aspect.1, self.pixel_aspect.0)
        } else {
            self.pixel_aspect
        }
    }
    /// The picture's own crop borders as pixel insets, `[0; 4]` when the track
    /// states none it can keep.
    pub fn insets(&self) -> [u32; 4] {
        let [l, t, r, b] = self.insets;
        match self.rotation {
            90 => [b, l, t, r],
            180 => [r, b, l, t],
            270 => [t, r, b, l],
            _ => self.insets,
        }
    }
    pub fn codec(&self) -> &'static str {
        self.codec
    }
    /// The signal the track's `Colour` element states, zeroes where the muxer
    /// wrote none or wrote only some of the three codes.
    pub fn colour(&self) -> ColourDescription {
        self.colour
    }
    /// The mastering display and light level the track names, empty where it
    /// names none. A tone map needs these and no AV1 or VP9 track header beyond
    /// the `Colour` element carries them.
    pub fn hdr(&self) -> HdrMetadata {
        self.hdr
    }
    /// The signal the coding states for itself: an AV1 sequence header's colour
    /// description, which the reader takes from the track's own bytes when it
    /// opens — the CodecPrivate configuration OBUs and the first packet — and
    /// afterwards from whatever the decoder is currently holding. VP9 carries a
    /// colour matrix and a range flag in every frame header rather than an H.273
    /// triple, so it is left to the container.
    pub fn bitstream_colour(&self) -> ColourDescription {
        let live = match &self.decoder {
            VideoDecoder::Av1(d) => d
                .color()
                .map(crate::codec::av1_sequence::Color::signal)
                .unwrap_or_default(),
            VideoDecoder::Vp9(_) | VideoDecoder::Ffv1(_) => ColourDescription::default(),
            VideoDecoder::Avc(d) => d
                .active_vui()
                .or_else(|| d.recorded_vui())
                .and_then(|v| v.video_signal)
                .map(|(_, full_range, colour)| {
                    let [primaries, transfer, matrix] = colour.unwrap_or([0; 3]);
                    ColourDescription {
                        primaries,
                        transfer,
                        matrix,
                        full_range,
                    }
                })
                .unwrap_or_default(),
            VideoDecoder::Hevc(d) => d
                .parameters()
                .0
                .vui
                .as_ref()
                .and_then(|v| v.signal.as_ref())
                .map(|v| {
                    let [primaries, transfer, matrix] = v.colour.unwrap_or([0; 3]);
                    ColourDescription {
                        primaries,
                        transfer,
                        matrix,
                        full_range: v.full_range,
                    }
                })
                .unwrap_or_default(),
        };
        let seed = self.open_signal.map(|(seed, _)| seed).unwrap_or_default();
        live.filled_with(seed)
    }
    /// The light the coding names for itself: an AV1 stream's mastering display
    /// and content light level metadata OBUs, which are there in a file whose
    /// Matroska `Colour` element left them out — and, at that, in no box at all
    /// for the files that write them only inside a packet. The reader asks those
    /// bytes when it opens, so a caller that grades the first picture is not left
    /// asking a decoder that has not run yet.
    pub fn bitstream_hdr(&self) -> HdrMetadata {
        let live = match &self.decoder {
            VideoDecoder::Av1(d) => d.hdr(),
            VideoDecoder::Vp9(_) | VideoDecoder::Avc(_) | VideoDecoder::Ffv1(_) => {
                HdrMetadata::default()
            }
            VideoDecoder::Hevc(d) => d.hdr(),
        };
        let seed = self.open_signal.map(|(_, seed)| seed).unwrap_or_default();
        let mut hdr = live;
        hdr.merge(seed);
        hdr
    }
    /// The cap this reader sizes its RGB picture by, which is the cap a caller
    /// converting the planes it hands over has to size by too.
    pub fn rgb_budget(&self) -> usize {
        self.rgb_budget
    }
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }
    pub fn frame_period(&self) -> Duration {
        Duration::from_nanos((self.end - self.start) as u64)
    }
    pub fn clock_quantum(&self) -> u64 {
        self.demux.timestamp_scale_ns()
    }
    pub fn frame_interval(&self) -> Option<(u128, u128, u32)> {
        if self.frames == 0 {
            None
        } else {
            Some((self.start, self.end, 1_000_000_000))
        }
    }
    /// Timeline length relative to the first displayed video frame. When a
    /// streaming file omits Duration, estimate its tail from indexed packet PTS.
    pub fn cache_packet_count(&self) -> usize { self.demux.packets.len() }

    pub fn cache_packets(&self) -> Vec<(u64, u64, Duration, Duration)> {
        let origin = self.origin();
        let mut packets: Vec<_> = self.demux.packets.iter()
            .filter(|p| p.track == self.track && !p.invisible).collect();
        packets.sort_unstable_by_key(|p| p.pts_ns);
        packets.iter().enumerate().filter_map(|(index, p)| {
            let start = p.pts_ns.checked_sub(origin)?;
            // SimpleBlocks commonly omit duration. The next presentation PTS
            // defines the displayed interval, including variable frame rates.
            let end = if let Some(duration) = p.duration_ns {
                start.checked_add(i64::try_from(duration).ok()?)?
            } else if let Some(next) = packets.get(index + 1) {
                next.pts_ns.checked_sub(origin)?
            } else {
                start.checked_add(self.frame_period().as_nanos() as i64)?
            };
            (end > 0).then(|| (p.offset, p.offset.saturating_add(p.size as u64),
                Duration::from_nanos(start.max(0) as u64), Duration::from_nanos(end as u64)))
        }).collect()
    }

    pub fn duration(&self) -> Option<Duration> {
        let mut timestamps = self
            .demux
            .packets
            .iter()
            .filter(|p| p.track == self.track && !p.invisible)
            .map(|p| p.pts_ns);
        let first = timestamps.next()?;
        let origin = self.origin();
        let end = if let Some(declared) = self.demux.duration_ns {
            i128::from(declared)
        } else {
            let mut last = first;
            let mut interval = 33_333_333i128;
            for pts in timestamps {
                if pts > last {
                    interval = i128::from(pts) - i128::from(last);
                    last = pts;
                }
            }
            i128::from(last) + interval
        };
        let nanos = u64::try_from(end - i128::from(origin)).ok()?;
        (nanos > 0).then(|| Duration::from_nanos(nanos))
    }
    pub fn rewind(&mut self) {
        match &mut self.decoder {
            VideoDecoder::Avc(d) => d.reset(),
            VideoDecoder::Hevc(d) => d.reset(),
            VideoDecoder::Vp9(d) => d.reset(),
            VideoDecoder::Av1(d) => d.reset(),
            VideoDecoder::Ffv1(d) => d.reset(),
        };
        self.index = 0;
        self.pending = None;
        self.queued.clear();
        self.base = None;
        self.start = 0;
        self.end = 0;
        self.frames = 0;
        self.failed = false;
        self.last_duration = 33_333_333;
    }
    /// Where the timeline starts for the reader: the first frame once one has
    /// been displayed, and the first packet of the track before that, which is
    /// the same fallback `duration` uses.
    fn origin(&self) -> i64 {
        self.base.unwrap_or_else(|| {
            if let Some(pts) = self
                .future_pts
                .as_ref()
                .and_then(|v| v.first())
                .filter(|v| **v != i64::MAX)
            {
                return *pts;
            }
            self.demux
                .packets
                .iter()
                .find(|p| p.track == self.track && !p.invisible)
                .map_or(0, |p| p.pts_ns)
        })
    }
    /// Restart decoding at the keyframe that opens the stretch of timeline
    /// containing `target_ns`, counted where `frame_interval` counts, and report
    /// the position landed on. Blocks are indexed in decode order, which a
    /// reordered track need not state in, so the scan takes the latest keyframe
    /// rather than the last one it meets; a target in front of every keyframe
    /// restarts the track at its first block.
    pub fn seek_to_sync(&mut self, target_ns: i64) -> Result<i64> {
        // A lazy index can initially contain only decode-only pre-roll. Find
        // the first displayed timestamp before pinning the seek origin.
        while self.base.is_none()
            && !self
                .demux
                .packets
                .iter()
                .any(|p| p.track == self.track && !p.invisible)
            && self.demux.scan_more()?
        {}
        let origin = self.origin();
        let target = target_ns.saturating_add(origin);
        // A jump needs the blocks behind its target as well as the ones in
        // front of it, which is the one question lazy indexing cannot dodge —
        // and the walk stops at the target, so the price is the distance
        // travelled rather than the length of the item.
        self.demux.scan_until(target)?;
        let mut chosen: Option<(usize, i64)> = None;
        for (index, packet) in self.demux.packets.iter().enumerate() {
            if packet.track != self.track || !packet.keyframe || packet.pts_ns > target {
                continue;
            }
            if chosen.is_none_or(|(_, at)| packet.pts_ns >= at) {
                chosen = Some((index, packet.pts_ns));
            }
        }
        let chosen = chosen.or_else(|| {
            let index = self
                .demux
                .packets
                .iter()
                .position(|p| p.track == self.track)?;
            Some((index, self.demux.packets[index].pts_ns))
        });
        let Some((index, pts)) = chosen else {
            return Ok(0);
        };
        match &mut self.decoder {
            VideoDecoder::Avc(d) => d.reset(),
            VideoDecoder::Hevc(d) => d.reset(),
            VideoDecoder::Vp9(d) => d.reset(),
            VideoDecoder::Av1(d) => d.reset(),
            VideoDecoder::Ffv1(d) => d.reset(),
        };
        // The picture held over from before belongs to the stretch being left
        // behind, and a decoder that had just failed is the one rebuilt here.
        self.pending = None;
        self.queued.clear();
        self.failed = false;
        self.index = index;
        // Pinning the origin matters only before the first frame: without it the
        // keyframe jumped to would become the start of the timeline and the
        // position would read back as zero.
        self.base = Some(origin);
        let landed = i64::try_from(i128::from(pts) - i128::from(origin))
            .unwrap_or(0)
            .max(0);
        self.start = u128::try_from(landed).unwrap_or_default();
        self.end = self.start;
        Ok(landed)
    }
    /// Seek and land on the frame covering `target_ns`, refreshing `rgb()` the
    /// way continuous playback does. The pre-roll costs what decoding it costs.
    pub fn seek_to_frame(&mut self, target_ns: i64) -> Result<()> {
        self.seek_to_sync(target_ns)?;
        let target = u128::try_from(target_ns.max(0)).unwrap_or(u128::MAX);
        let result = (|| {
            while self.read_inner()? {
                if self.end > target {
                    return Ok(());
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_decoded(&mut self) -> Result<Option<Frame>> {
        if self.future_pts.is_none() {
            return self.decode_next();
        }
        loop {
            if let Some((index, frame)) = self.queued.iter().enumerate().min_by_key(|(_, f)| f.pts)
            {
                let future = self.future_pts.as_ref().and_then(|v| v.get(self.index));
                if future.is_none_or(|pts| frame.pts <= *pts) {
                    return Ok(Some(self.queued.remove(index)));
                }
            }
            let Some(frame) = self.decode_next()? else {
                return Ok(self
                    .queued
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, f)| f.pts)
                    .map(|(i, _)| i)
                    .map(|i| self.queued.remove(i)));
            };
            let mut bytes = 0usize;
            for frame in self.queued.iter().chain(std::iter::once(&frame)) {
                if let Picture::Coded(p, _) = &frame.decoded {
                    for plane in [&p.y, &p.cb, &p.cr] {
                        bytes = plane
                            .len()
                            .checked_mul(2)
                            .and_then(|n| bytes.checked_add(n))
                            .ok_or_else(|| invalid("Matroska reorder storage overflow"))?;
                    }
                }
            }
            if bytes > self.queue_budget {
                return Err(invalid("Matroska reordered frames exceed memory budget"));
            }
            self.queued
                .try_reserve(1)
                .map_err(|_| invalid("cannot allocate Matroska reorder queue"))?;
            self.queued.push(frame);
        }
    }
    fn decode_next(&mut self) -> Result<Option<Frame>> {
        // The index only holds the clusters walked so far, so a reader that has
        // run out of blocks asks the demuxer for the next one before it believes
        // the item has ended.
        while self.index < self.demux.packets.len() || self.demux.scan_more()? {
            let index = self.index;
            self.index += 1;
            if self.demux.packets[index].track != self.track {
                continue;
            }
            let pts = self.demux.packets[index].pts_ns;
            let duration = self.demux.packets[index].duration_ns;
            let packet = self.demux.read_packet(index)?;
            let mut visible = None;
            match &mut self.decoder {
                VideoDecoder::Avc(d) => {
                    if let Some(picture) = d.decode_order(&packet)? {
                        let colour =
                            AvcColour::from_vui(d.active_vui().or_else(|| d.recorded_vui()))?;
                        visible = Some(Frame {
                            decoded: Picture::Coded(picture, colour),
                            pts,
                            duration,
                        });
                    }
                }
                VideoDecoder::Hevc(d) => {
                    if let Some(frame) = d.decode_packet(&packet)?.filter(|f| f.output) {
                        let p = &frame.picture;
                        let colour = AvcColour::from_hevc_vui(d.parameters().0.vui.as_ref())?;
                        let picture = IntraPicture {
                            coded_width: p.dimensions[0] as usize,
                            coded_height: p.dimensions[1] as usize,
                            crop: p.crop.map(|v| v as usize),
                            bit_depth: p.depth[0],
                            y: p.planes[0].samples().to_vec(),
                            cb: p.planes[1].samples().to_vec(),
                            cr: p.planes[2].samples().to_vec(),
                        };
                        visible = Some(Frame {
                            decoded: Picture::Coded(Arc::new(picture), colour),
                            pts,
                            duration,
                        });
                    }
                }
                VideoDecoder::Ffv1(d) => {
                    let decoded = d.decode(&packet)?;
                    let (kr, kb) = match self.colour.matrix {
                        0 | 2 | 5 | 6 => (0.299, 0.114),
                        1 => (0.2126, 0.0722),
                        7 => (0.212, 0.087),
                        9 => (0.2627, 0.0593),
                        _ => return Err(invalid("unsupported FFV1 display colour matrix")),
                    };
                    let colour = AvcColour {
                        kr,
                        kb,
                        full: self.colour.full_range,
                    };
                    let p = PackedPlanar::new(decoded.frame, decoded.depth, colour)?;
                    visible = Some(Frame {
                        decoded: Picture::Ffv1(Arc::new(p)),
                        pts,
                        duration,
                    });
                }
                VideoDecoder::Vp9(d) => {
                    for frame in vp9::frames(&packet)? {
                        let decoded = d.decode(frame)?;
                        if decoded.header.show_frame {
                            if visible.is_some() {
                                return Err(invalid(
                                    "multiple visible frames in one WebM packet need distinct timestamps",
                                ));
                            }
                            visible = Some(Frame {
                                decoded: Picture::Vp9(Box::new(decoded)),
                                pts,
                                duration,
                            });
                        }
                    }
                }
                VideoDecoder::Av1(d) => {
                    for decoded in d.decode_packet(&packet)? {
                        if decoded.show {
                            if visible.is_some() {
                                return Err(invalid(
                                    "multiple visible frames in one WebM packet need distinct timestamps",
                                ));
                            }
                            visible = Some(Frame {
                                decoded: Picture::Av1(decoded),
                                pts,
                                duration,
                            });
                        }
                    }
                }
            }
            // Invisible blocks are still fully decoded: later frames may
            // reference them. Only their presentation is suppressed.
            if visible.is_some() && !self.demux.packets[index].invisible {
                return Ok(visible);
            }
        }
        if let VideoDecoder::Av1(d) = &mut self.decoder {
            d.finish()?;
        }
        Ok(None)
    }
    pub fn read_frame(&mut self) -> Result<bool> {
        if self.failed {
            return Err(invalid("WebM playback requires rewind after an error"));
        }
        let result = self.read_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_inner(&mut self) -> Result<bool> {
        let Some(current) = self.advance()? else {
            return Ok(false);
        };
        self.convert_to_rgb(&current)?;
        if self.rotation != 0 {
            self.rgb = crate::playback_native::rotate_plane(
                &self.rgb,
                current.size[0] as usize,
                current.size[1] as usize,
                self.rotation,
                3,
            );
        }
        Ok(true)
    }
    /// Advances to the next visible frame and updates timing/geometry state.
    /// The decoded picture is returned for the caller to convert.
    fn advance(&mut self) -> Result<Option<CurrentFrame>> {
        let current = match self.pending.take() {
            Some(frame) => Some(frame),
            None => self.next_decoded()?,
        };
        let Some(current) = current else {
            return Ok(None);
        };
        let next = self.next_decoded()?;
        let inferred_duration = if let Some(next) = &next {
            u64::try_from(
                next.pts
                    .checked_sub(current.pts)
                    .filter(|&v| v > 0)
                    .ok_or_else(|| invalid("non-increasing WebM presentation timestamps"))?,
            )
            .map_err(|_| invalid("WebM duration overflow"))?
        } else {
            self.default_duration.unwrap_or(self.last_duration)
        };
        let duration = match (current.duration, next.is_some()) {
            (Some(explicit), true) => explicit.min(inferred_duration),
            (Some(explicit), false) => explicit,
            (None, _) => inferred_duration,
        };
        if duration == 0 {
            return Err(invalid("zero Matroska frame duration"));
        }
        let base = *self.base.get_or_insert(current.pts);
        let start = u128::try_from(i128::from(current.pts) - i128::from(base))
            .map_err(|_| invalid("WebM presentation timestamp precedes origin"))?;
        let (size, depth, full_range, color_space, monochrome) = match &current.decoded {
            Picture::Coded(p, colour) => {
                let (w, h) = p.dimensions();
                ([w as u32, h as u32], p.bit_depth, colour.full, 0, false)
            }
            Picture::Ffv1(p) => (
                [p.frame.width as u32, p.frame.height as u32],
                p.depth,
                p.colour.full,
                0,
                false,
            ),
            Picture::Vp9(d) => (
                d.picture.size,
                d.picture.depth,
                d.header.picture.format.full_range,
                d.header.picture.format.color_space,
                false,
            ),
            Picture::Av1(d) => {
                let color_space = match d.color.matrix {
                    1 => 2,
                    2 | 5 | 6 => 1,
                    7 => 4,
                    9 => 5,
                    _ => return Err(invalid("unsupported AV1 RGB matrix coefficients")),
                };
                (
                    d.picture.size,
                    d.picture.depth,
                    d.color.full_range,
                    color_space,
                    d.color.monochrome,
                )
            }
        };
        let w = size[0] as usize;
        let h = size[1] as usize;
        self.pending = next;
        self.dimensions = if matches!(self.rotation, 90 | 270) {
            [h, w]
        } else {
            [w, h]
        };
        self.start = start;
        self.end = start + u128::from(duration);
        self.last_duration = duration;
        self.frames += 1;
        Ok(Some(CurrentFrame {
            decoded: current.decoded,
            size,
            depth,
            full_range,
            color_space,
            monochrome,
        }))
    }
    fn convert_to_rgb(&mut self, current: &CurrentFrame) -> Result<bool> {
        if let Picture::Ffv1(p) = &current.decoded {
            p.to_rgb(&mut self.rgb, self.rgb_budget)?;
            return Ok(true);
        }
        if let Picture::Coded(p, colour) = &current.decoded {
            crate::playback_native::avc_to_rgb(p, *colour, &mut self.rgb, self.rgb_budget)?;
            return Ok(true);
        }
        let (kr, kb) = match current.color_space {
            0 | 1 | 3 => (0.299, 0.114),
            2 => (0.2126, 0.0722),
            4 => (0.212, 0.087),
            5 => (0.2627, 0.0593),
            _ => return Err(invalid("unsupported native RGB colour configuration")),
        };
        let w = current.size[0] as usize;
        let h = current.size[1] as usize;
        let len = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(3))
            .filter(|&v| v <= self.rgb_budget)
            .ok_or_else(|| invalid("WebM RGB frame exceeds budget"))?;
        if self.rgb.len() != len {
            self.rgb = crate::buffer(len)?;
        }
        let scale = f64::from(1u32 << (current.depth - 8));
        let (offset, yr, cr) = if current.full_range {
            (
                0.0,
                f64::from((1u32 << current.depth) - 1),
                f64::from((1u32 << current.depth) - 1),
            )
        } else {
            (16.0 * scale, 219.0 * scale, 224.0 * scale)
        };
        let planes = match &current.decoded {
            Picture::Coded(_, _) | Picture::Ffv1(_) => unreachable!("coded planes handled above"),
            Picture::Vp9(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
            Picture::Av1(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
        };
        let [y, u, v] = planes;
        let y_stride = y.1;
        let u_stride = u.1;
        let v_stride = v.1;
        let green = 1.0 - kr - kb;
        for py in 0..h {
            let luma_row = py * y_stride;
            let cb_row = (py / 2) * u_stride;
            let cr_row = (py / 2) * v_stride;
            let line = &mut self.rgb[py * w * 3..][..w * 3];
            for (px, pixel) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let luma = (f64::from(y.0[luma_row + px]) - offset) / yr;
                let cb = (f64::from(u.0[cb_row + px / 2]) - 128.0 * scale) / cr;
                let cv = (f64::from(v.0[cr_row + px / 2]) - 128.0 * scale) / cr;
                let (cb, cv) = if current.monochrome {
                    (0.0, 0.0)
                } else {
                    (cb, cv)
                };
                let r = luma + 2.0 * (1.0 - kr) * cv;
                let b = luma + 2.0 * (1.0 - kb) * cb;
                let g = (luma - kr * r - kb * b) / green;
                for (out, sample) in pixel.iter_mut().zip([r, g, b]) {
                    *out = (sample * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Ok(true)
    }
    /// Decode the next visible frame and return it as packed 8-bit planes for
    /// GPU conversion (no CPU RGB pass). Planes retain coded orientation;
    /// callers apply `rotation()`. `rgb()` is not updated by this call.
    pub fn read_frame_planes(&mut self) -> Result<Option<Planar8>> {
        if self.failed {
            return Err(invalid("WebM playback requires rewind after an error"));
        }
        let result = self.read_planes_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    /// Keep AVC/HEVC/FFV1 sample depth for export and off-thread rendering.
    pub fn read_frame_raw(&mut self) -> Result<Option<crate::playback_native::RawFrame>> {
        if self.failed {
            return Err(invalid("WebM playback requires rewind after an error"));
        }
        let result = (|| {
            let Some(current) = self.advance()? else {
                return Ok(None);
            };
            let raw = match &current.decoded {
                Picture::Ffv1(p) => crate::playback_native::RawFrame::Planar(p.clone()),
                Picture::Coded(picture, colour) => crate::playback_native::RawFrame::Avc {
                    picture: picture.clone(),
                    colour: *colour,
                },
                Picture::Vp9(_) | Picture::Av1(_) if current.depth > 8 => {
                    let planes=match &current.decoded {
                        Picture::Vp9(d)=>d.picture.planes.each_ref().map(|p|(p.samples.as_slice(),p.width)),
                        Picture::Av1(d)=>d.picture.planes.each_ref().map(|p|(p.samples.as_slice(),p.width)),
                        _=>unreachable!(),
                    };
                    let [w,h]=current.size.map(|n|n as usize);
                    let cw=w.div_ceil(2);let ch=h.div_ceil(2);
                    let bytes=w.checked_mul(h).and_then(|n|cw.checked_mul(ch).and_then(|c|c.checked_mul(2)).and_then(|c|n.checked_add(c))).and_then(|n|n.checked_mul(2))
                        .filter(|&n|n<=self.rgb_budget).ok_or_else(||invalid("packed WebM planes exceed budget"))?;
                    let mut data=Vec::with_capacity(bytes);
                    for (index,(samples,stride)) in planes.into_iter().enumerate() {
                        let (width,height)=if index==0 {(w,h)}else{(cw,ch)};
                        for row in 0..height {for col in 0..width {
                            let value=if index!=0 && current.monochrome {1u16<<(current.depth-1)}else {
                                *samples.get(row*stride+col).ok_or_else(||invalid("incomplete WebM sample plane"))?
                            };
                            data.extend_from_slice(&value.to_le_bytes());
                        }}
                    }
                    let (kr,kb)=match current.color_space {0|1|3=>(0.299,0.114),2=>(0.2126,0.0722),4=>(0.212,0.087),5=>(0.2627,0.0593),_=>return Err(invalid("unsupported native RGB colour configuration"))};
                    crate::playback_native::RawFrame::Planar(Arc::new(PackedPlanar::new(
                        crate::native_geometry::GeometryFrame{width:w,height:h,subsampling:Some([2,2]),data},current.depth,
                        AvcColour{kr,kb,full:current.full_range})?))
                },
                _ => crate::playback_native::RawFrame::Planar8(Arc::new(
                    self.current_planes(&current)?,
                )),
            };
            Ok(Some(raw))
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_planes_inner(&mut self) -> Result<Option<Planar8>> {
        let Some(current) = self.advance()? else {
            return Ok(None);
        };
        self.current_planes(&current).map(Some)
    }
    fn current_planes(&self, current: &CurrentFrame) -> Result<Planar8> {
        if let Picture::Ffv1(p) = &current.decoded {
            return p.to_planar8(self.rgb_budget);
        }
        if let Picture::Coded(p, colour) = &current.decoded {
            let (w, h) = p.dimensions();
            let bytes = w
                .checked_mul(h)
                .and_then(|n| n.checked_mul(3))
                .map(|n| n / 2)
                .ok_or_else(|| invalid("Matroska plane size overflow"))?;
            if bytes > self.rgb_budget {
                return Err(invalid("WebM video planes exceed budget"));
            }
            return Ok(crate::playback_native::avc_to_planar8(p, *colour));
        }
        let (kr, kb) = match current.color_space {
            0 | 1 | 3 => (0.299, 0.114),
            2 => (0.2126, 0.0722),
            4 => (0.212, 0.087),
            5 => (0.2627, 0.0593),
            _ => return Err(invalid("unsupported native RGB colour configuration")),
        };
        let w = current.size[0] as usize;
        let h = current.size[1] as usize;
        let shift = current.depth.saturating_sub(8);
        let chroma_width = w.div_ceil(2);
        let chroma_height = h.div_ceil(2);
        let chroma_bytes = chroma_width
            .checked_mul(chroma_height)
            .ok_or_else(|| invalid("WebM chroma size overflow"))?;
        let planes_bytes = w
            .checked_mul(h)
            .and_then(|v| v.checked_add(chroma_bytes))
            .and_then(|v| v.checked_add(chroma_bytes))
            .ok_or_else(|| invalid("WebM plane size overflow"))?;
        if planes_bytes > self.rgb_budget {
            return Err(invalid("WebM video planes exceed budget"));
        }
        let narrow = |plane: &[u16], stride: usize, width: usize, height: usize| -> Vec<u8> {
            let mut out = Vec::with_capacity(width * height);
            for row in 0..height {
                out.extend(
                    plane[row * stride..][..width]
                        .iter()
                        .map(|&v| (v >> shift) as u8),
                );
            }
            out
        };
        let planes = match &current.decoded {
            Picture::Coded(_, _) | Picture::Ffv1(_) => unreachable!("coded planes handled above"),
            Picture::Vp9(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
            Picture::Av1(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
        };
        let [y, u, v] = planes;
        let y_plane = narrow(y.0, y.1, w, h);
        let (cb, cr) = if current.monochrome {
            let fill = if current.full_range { 0u8 } else { 128u8 };
            (
                vec![fill; chroma_width * chroma_height],
                vec![fill; chroma_width * chroma_height],
            )
        } else {
            (
                narrow(u.0, u.1, chroma_width, chroma_height),
                narrow(v.0, v.1, chroma_width, chroma_height),
            )
        };
        Ok(Planar8 {
            width: w,
            height: h,
            chroma_width,
            chroma_height,
            y: y_plane,
            cb,
            cr,
            colour: AvcColour {
                kr,
                kb,
                full: current.full_range,
            },
        })
    }
}

/// Compatibility name retained for callers of the original VP9-only adapter.
pub type WebmVp9Reader<R> = WebmVideoReader<R>;

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    #[test]
    fn av1_video_uses_native_reader_and_rewinds() {
        let data = include_bytes!("../../../tests/fixtures/av1/random-access.webm");
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        assert!(reader.duration().is_some());
        let mut first = Vec::new();
        let mut count = 0;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [192, 128]);
            assert_eq!(reader.rgb().len(), 192 * 128 * 3);
            if count == 0 {
                first = reader.rgb().to_vec();
            }
            count += 1;
        }
        assert_eq!(count, 24);
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.rgb(), first);
    }
    #[test]
    fn cached_simpleblocks_follow_variable_presentation_intervals() {
        use super::*;
        let input = include_bytes!("../../../tests/fixtures/vp9/motion.webm");
        let mut reader = WebmVideoReader::open(Cursor::new(input), 16 << 20).unwrap();
        reader.demux.scan_all().unwrap();
        let mut time = 0;
        for (index, packet) in reader.demux.packets.iter_mut().filter(|p| p.track == reader.track && !p.invisible).enumerate() {
            packet.pts_ns = time;
            packet.duration_ns = None;
            time += if index % 2 == 0 { 33_000_000 } else { 67_000_000 };
        }
        let packets = reader.cache_packets();
        assert!(packets.len() > 2);
        for pair in packets.windows(2) { assert_eq!(pair[0].3, pair[1].2); }
        assert_eq!(packets[0].3 - packets[0].2, Duration::from_millis(33));
        assert_eq!(packets[1].3 - packets[1].2, Duration::from_millis(67));
    }

    #[test]
    fn duration_and_progress_use_video_origin_and_index_fallback() {
        use super::*;
        let input = include_bytes!("../../../tests/fixtures/vp9/motion.webm");
        let mut reader = WebmVideoReader::open(Cursor::new(input), 16 << 20).unwrap();
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        // The estimate below reads the item's own blocks, so it is made with
        // every block indexed.
        reader.demux.scan_all().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(
            reader.frame_interval(),
            Some((0, 100_000_000, 1_000_000_000))
        );
        // Files recorded by streaming writers may omit Segment Duration.
        reader.demux.duration_ns = None;
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        reader.rewind();
        for packet in &mut reader.demux.packets {
            packet.pts_ns += 7_000_000;
        }
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        reader.demux.duration_ns = Some(1_007_000_000);
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        reader.rewind();
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
    }
    #[test]
    fn native_detection_timing_rgb_eof_and_rewind() {
        let input = include_bytes!("../../../tests/fixtures/vp9/motion.webm");
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
        assert!(reader.duration().is_some());
        let mut first = Vec::new();
        let mut count = 0;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [128, 96]);
            assert_eq!(reader.rgb().len(), 128 * 96 * 3);
            assert_eq!(
                reader.frame_interval(),
                Some((
                    count * 100_000_000,
                    (count + 1) * 100_000_000,
                    1_000_000_000
                ))
            );
            if count == 0 {
                first = reader.rgb().to_vec();
            }
            count += 1;
        }
        assert_eq!(count, 10);
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.rgb(), first);
        assert_eq!(
            reader.frame_interval(),
            Some((0, 100_000_000, 1_000_000_000))
        );
    }

    /// Every frame of a fixture in order: the start of its interval and the RGB
    /// continuous playback shows for it.
    fn shown_frames(input: &'static [u8]) -> Vec<(u128, Vec<u8>)> {
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        let mut frames = Vec::new();
        while reader.read_frame().unwrap() {
            let (start, _, _) = reader.frame_interval().unwrap();
            frames.push((start, reader.rgb().to_vec()));
        }
        frames
    }

    /// Seeking restarts the decoder at the keyframe before the target and runs
    /// it forward, so the picture that lands on the target has to be the one
    /// continuous playback shows there: a decoder left holding references from
    /// the stretch behind would show a different one. The fixture is 10 frames
    /// at 100 ms across three clusters, so a target inside the second cluster is
    /// reached by decoding from its own keyframe rather than from the start.
    #[test]
    fn a_seek_lands_on_the_frame_continuous_playback_shows() {
        use super::Duration;
        let input = include_bytes!("../../../tests/fixtures/vp9/motion.webm");
        let forward = shown_frames(input);
        assert_eq!(forward.len(), 10);
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
        assert!(reader.seekable());
        for target in [0u64, 50, 400, 600, 850] {
            reader.seek(Duration::from_millis(target)).unwrap();
            let at = forward
                .iter()
                .position(|(start, _)| *start > u128::from(target) * 1_000_000)
                .unwrap_or(forward.len())
                - 1;
            let (start, _, _) = reader.frame_interval().unwrap();
            assert_eq!(
                (start, reader.rgb()),
                (forward[at].0, forward[at].1.as_slice()),
                "seek to {target} ms"
            );
        }
        // The cursor resumes just after the frame landed on, not where the
        // keyframe behind it started.
        assert!(reader.read_frame().unwrap());
        assert_eq!(
            reader.frame_interval(),
            Some((900_000_000, 1_000_000_000, 1_000_000_000))
        );
        // Past the last frame there is nothing to land on but the last frame.
        reader.seek(Duration::from_secs(9)).unwrap();
        assert_eq!(
            reader.frame_interval(),
            Some((900_000_000, 1_000_000_000, 1_000_000_000))
        );
        assert!(!reader.read_frame().unwrap());
    }

    /// A seek is also how `--start-time` opens a file, before a single frame has
    /// been shown. The keyframe jumped to must not become the start of the
    /// timeline: position still counts from the first frame of the track.
    #[test]
    fn seeking_before_the_first_frame_keeps_the_track_origin() {
        use super::Duration;
        let input = include_bytes!("../../../tests/fixtures/vp9/motion.webm");
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
        reader.seek(Duration::from_millis(600)).unwrap();
        assert_eq!(
            reader.frame_interval(),
            Some((600_000_000, 700_000_000, 1_000_000_000))
        );
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
    }

    /// The fixture is a 16×16 VP9 picture with `PixelCropLeft/Top/Right/Bottom`
    /// of two pixels each, so ffmpeg's own decode of it reports the frame side
    /// data `Frame Cropping: 2/2/2/2`:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=16x16:rate=4:duration=1 \
    ///   -c:v libvpx-vp9 -crf 63 -b:v 0 -pix_fmt yuv420p -an \
    ///   -write_crc32 0 tests/fixtures/display/crops.mkv
    /// ```
    ///
    /// No ffmpeg version writes the crop elements themselves, so the file is
    /// that one with four `54 cc/bb/dd/aa 84 00000002` elements inserted at the
    /// end of the `Video` element and the sizes of `Video`, `TrackEntry`,
    /// `Tracks` and `Segment` grown to hold them; `-write_crc32 0` is what
    /// leaves those sizes widen-able in place. The reader's side of the
    /// contract: the crop reaches the player as insets, the coded frame keeps
    /// its full size, and two-pixel-square drawing of a 16×16 coded frame
    /// states square pixels rather than a stretched ones.
    #[test]
    fn a_cropped_webm_reaches_the_player_with_its_borders_and_full_frames() {
        let input = include_bytes!("../../../tests/fixtures/display/crops.mkv");
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        assert_eq!(reader.insets(), [2, 2, 2, 2]);
        assert_eq!(reader.pixel_aspect(), (1, 1));
        assert_eq!(reader.video_codec(), "VP9");
        let mut count = 0;
        while reader.read_frame().unwrap() {
            // Decoding is unaffected: the frame arrives coded, borders and all.
            assert_eq!(reader.dimensions(), [16, 16]);
            assert_eq!(reader.rgb().len(), 16 * 16 * 3);
            count += 1;
        }
        assert_eq!(count, 4);
    }

    /// One cluster for the whole file, so every seek decodes the entire
    /// pre-roll. The point is that a rebuilt AV1 decoder still arrives at the
    /// same pictures, wherever in the file it was asked to start.
    #[test]
    fn seeking_a_single_cluster_av1_file_replays_the_same_pictures() {
        use super::Duration;
        let input = include_bytes!("../../../tests/fixtures/av1/random-access.webm");
        let forward = shown_frames(input);
        assert_eq!(forward.len(), 24);
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        assert!(reader.seekable());
        for at in [forward.len() / 2, forward.len() - 1] {
            let target = u64::try_from(forward[at].0).unwrap() + 1;
            reader.seek(Duration::from_nanos(target)).unwrap();
            assert_eq!(
                (reader.frame_interval().unwrap().0, reader.rgb()),
                (forward[at].0, forward[at].1.as_slice()),
                "seek to frame {at}"
            );
        }
    }

    /// A file that states its light in no box at all: SVT-AV1 4.2.0 wrote a
    /// BT.2020 triple and a PQ curve into its sequence header, the BT.2020
    /// mastering volume and a 1 234/567 content light level into metadata OBUs
    /// inside the first packet, and the muxer put in the track's `Colour` element
    /// one range flag and nothing else — measured from this file's own bytes,
    /// which hold no `MasteringMetadata`, no `MaxCLL`, no primaries and no curve.
    /// So the whole HDR answer belongs to the coding, and a reader that grades
    /// the first picture needs it the moment the file is open.
    #[test]
    fn a_track_that_writes_its_light_only_in_band_answers_at_open() {
        use super::*;
        let input = include_bytes!("../../../tests/fixtures/av1/hdr-in-band.mkv");
        let mut reader = WebmVideoReader::open(Cursor::new(input.as_slice()), 16 << 20).unwrap();
        // The container's half, which is the range flag it wrote and nothing more.
        assert_eq!(reader.colour(), ColourDescription::default());
        assert!(reader.hdr().is_empty());
        // The coding's half, before a single picture has been decoded. Its
        // range flag is the one place the two halves of this real file point
        // apart: the muxer wrote `55 b9 81 01`, which Matroska reads as limited,
        // while the sequence header SVT-AV1 wrote from the same `color-range=1`
        // says full. Limited is the value silence already has, so nothing
        // competes with the coding's triple and the composed answer takes its
        // full range — which is what the player's own test of these bytes pins.
        assert_eq!(
            reader.bitstream_colour(),
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            }
        );
        let light = reader.bitstream_hdr();
        assert!(light.mastering.unwrap().is_hdr10());
        assert_eq!(
            (light.light.max_cll, light.light.max_fall),
            (1_234.0, 567.0)
        );
        // The answer a player asks, which is the two halves composed: HDR enough
        // to grade, with the volume a tone map rolls a shoulder against.
        let player =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        assert!(player.colour().is_hdr());
        assert!(!player.hdr().is_empty());
        // And the same picture route that met these bytes at open still decodes
        // them, so a seek or a walk states the light it stated at open. The
        // encoder was asked for three pictures but its source ran out 0.35
        // seconds into a 3 fps rate, so the file holds two.
        let mut frames = 0;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [64, 64]);
            frames += 1;
        }
        assert_eq!(frames, 2);
        assert_eq!(reader.bitstream_hdr(), light);
    }
}
