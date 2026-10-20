//! Opus multistream (surround) — port of the core of
//! `src/opus_multistream_{encoder,decoder}.c`. Wraps N mono/coupled Opus
//! coders behind a channel-mapping layout so >2-channel audio (quad, 5.1,
//! 7.1) can be coded as a set of standard Opus streams concatenated with the
//! self-delimited framing.
//!
//! The channel bitrate allocation here is a simple even split across streams
//! (coupled streams get 2x a mono stream's share) — libopus adds a
//! surround-masking analysis on top, a quality refinement, not a conformance
//! requirement. The bitstream layout, mapping, and per-stream Opus coding are
//! standard, so streams interoperate with libopus.

use crate::{Error, Result};

use crate::encoder::MAX_ENCODING_DEPTH;
use crate::repacketizer::{Repacketizer, take_self_delimited_into};
use crate::soft_clip::{float_to_i16, i16_to_float};
use crate::{Application, Bandwidth, OpusDecoder, OpusEncoder};

/// Validate concatenated elementary packets and return their common duration
/// in samples at the mandatory 48 kHz Opus clock. No entropy decoding or copies.
pub fn multistream_packet_samples(mut data: &[u8], streams: usize) -> Result<usize> {
    if streams == 0 || streams > 255 || data.len() > i32::MAX as usize {
        return Err(Error::InvalidPacket("invalid multistream packet geometry"));
    }
    let mut duration = None;
    for stream in 0..streams {
        let (toc, frames, consumed) =
            crate::repacketizer::parse_packet(data, stream + 1 < streams)?;
        let samples = crate::repacketizer::samples_per_frame(toc, 48000) as usize * frames.len();
        if duration.is_some_and(|previous| previous != samples) {
            return Err(Error::InvalidPacket(
                "multistream packet durations disagree",
            ));
        }
        duration = Some(samples);
        data = data
            .get(consumed..)
            .ok_or(Error::InvalidPacket("truncated multistream packet"))?;
    }
    if !data.is_empty() {
        return Err(Error::InvalidPacket("trailing multistream packet data"));
    }
    Ok(duration.unwrap())
}

/// Vorbis channel layout for mapping family 1, channels 1..=8:
/// (nb_streams, nb_coupled_streams, channel_mapping).
const VORBIS_MAPPINGS: [(usize, usize, &[u8]); 8] = [
    (1, 0, &[0]),                      // mono
    (1, 1, &[0, 1]),                   // stereo
    (2, 1, &[0, 2, 1]),                // 1-d (3.0)
    (2, 2, &[0, 1, 2, 3]),             // quad
    (3, 2, &[0, 4, 1, 2, 3]),          // 5.0
    (4, 2, &[0, 4, 1, 2, 3, 5]),       // 5.1
    (4, 3, &[0, 4, 1, 2, 3, 5, 6]),    // 6.1
    (5, 3, &[0, 6, 1, 2, 3, 4, 5, 7]), // 7.1
];

/// How a set of Opus streams becomes a set of output channels.
///
/// A multistream packet is several ordinary Opus streams concatenated. Some are
/// coded as coupled stereo pairs and carry two channels each, the rest carry
/// one, and the mapping says which of the resulting channels goes where in the
/// output. Coupled streams always come first, so the channels a decoder
/// produces are the coupled pairs in order, then the mono streams in order.
///
/// [`surround`](Self::surround) builds the standard layout for a channel count
/// and mapping family, which is how [`OpusMSEncoder`] and [`OpusMSDecoder`]
/// obtain theirs. The fields are public so a caller can read what a given
/// channel count works out to, which is what the
/// [`OpusHead`](crate::OpusHead) for such a stream has to declare.
#[derive(Debug, Clone)]
pub struct ChannelLayout {
    /// The mapping family this layout came from: 0 for mono/stereo, 1 for the
    /// Vorbis surround orders, or 255 for discrete channels. Kept so the layout can describe itself to an
    /// [`OpusHead`](crate::OpusHead) without being asked twice.
    pub mapping_family: u8,
    /// Output channels this layout produces, which is what a caller interleaves.
    pub nb_channels: usize,
    /// Opus streams in the packet, coupled and uncoupled together.
    pub nb_streams: usize,
    /// How many of those streams are coupled stereo pairs. They are the first
    /// `nb_coupled_streams` of them, and each carries two channels, so the
    /// streams carry `nb_coupled_streams + nb_streams` channels in total.
    pub nb_coupled_streams: usize,
    /// For each output channel, which of the streams' channels feeds it.
    ///
    /// Indices `0..2 * nb_coupled_streams` are the coupled pairs, left then
    /// right; the rest are the mono streams in order. The value 255 marks a
    /// channel that is left silent. Length is `nb_channels`.
    pub mapping: Vec<u8>,
}

impl ChannelLayout {
    /// Validate stream geometry independently of decoder allocation.
    pub fn validate(&self) -> Result<()> {
        if !(1..=255).contains(&self.nb_channels)
            || !(1..=255).contains(&self.nb_streams)
            || self.nb_coupled_streams > self.nb_streams
            || self.nb_streams + self.nb_coupled_streams > 255
            || self.mapping.len() != self.nb_channels
            || self
                .mapping
                .iter()
                .any(|&v| v != 255 && usize::from(v) >= self.nb_streams + self.nb_coupled_streams)
        {
            return Err(Error::InvalidArgument("invalid multistream channel layout"));
        }
        Ok(())
    }

    /// Standard layout for a channel count + mapping family (0 = mono/stereo,
    /// 1 = Vorbis surround for 1..=8 channels, 255 = independent discrete channels).
    pub fn surround(channels: usize, mapping_family: u8) -> Result<Self> {
        match mapping_family {
            0 => {
                if channels == 1 {
                    Ok(ChannelLayout {
                        mapping_family: 0,
                        nb_channels: 1,
                        nb_streams: 1,
                        nb_coupled_streams: 0,
                        mapping: vec![0],
                    })
                } else if channels == 2 {
                    Ok(ChannelLayout {
                        mapping_family: 0,
                        nb_channels: 2,
                        nb_streams: 1,
                        nb_coupled_streams: 1,
                        mapping: vec![0, 1],
                    })
                } else {
                    Err(Error::InvalidArgument(
                        "family 0 supports only 1-2 channels",
                    ))
                }
            }
            1 => {
                if !(1..=8).contains(&channels) {
                    return Err(Error::InvalidArgument("family 1 supports 1-8 channels"));
                }
                let (ns, nc, m) = VORBIS_MAPPINGS[channels - 1];
                Ok(ChannelLayout {
                    mapping_family: 1,
                    nb_channels: channels,
                    nb_streams: ns,
                    nb_coupled_streams: nc,
                    mapping: m.to_vec(),
                })
            }
            255 => {
                if !(1..=255).contains(&channels) {
                    return Err(Error::InvalidArgument("family 255 supports 1-255 channels"));
                }
                Ok(ChannelLayout {
                    mapping_family: 255,
                    nb_channels: channels,
                    nb_streams: channels,
                    nb_coupled_streams: 0,
                    mapping: (0..channels).map(|c| c as u8).collect(),
                })
            }
            _ => Err(Error::InvalidArgument("unsupported mapping family")),
        }
    }

    /// Output-channel indices carrying `target` in the channel mapping.
    ///
    /// libopus walks these with a `-1`-sentinel cursor (`get_left_channel` and
    /// friends, one function per target); an iterator says the same thing once,
    /// without the sentinel.
    fn channels_for(&self, target: usize) -> impl Iterator<Item = usize> + '_ {
        self.mapping
            .iter()
            .enumerate()
            .filter_map(move |(i, &m)| (m as usize == target).then_some(i))
    }

    /// Mapping value for a coupled stream's left channel.
    fn left_target(stream_id: usize) -> usize {
        stream_id * 2
    }
    /// Mapping value for a coupled stream's right channel.
    fn right_target(stream_id: usize) -> usize {
        stream_id * 2 + 1
    }
    /// Mapping value for an uncoupled stream's single channel.
    fn mono_target(&self, stream_id: usize) -> usize {
        stream_id + self.nb_coupled_streams
    }
}

/// Multistream encoder: one Opus encoder per stream (coupled = stereo, the
/// rest mono), coded per the channel layout and concatenated self-delimited.
pub struct OpusMSEncoder {
    layout: ChannelLayout,
    encoders: Vec<OpusEncoder>,
    sample_rate: i32,
    /// Total target bitrate across all streams; read through
    /// [`bitrate_bps`](OpusMSEncoder::bitrate_bps), written through
    /// [`set_bitrate`](OpusMSEncoder::set_bitrate).
    ///
    /// Private because the value that matters is the per-stream split derived
    /// from it, not this number: a writable field here would let a caller set
    /// it and change nothing.
    bitrate_bps: i32,
    /// One stream's channels, de-interleaved out of the caller's input.
    buf_stream: Vec<f32>,
    /// One stream's coded packet, before it is framed into the output.
    buf_packet: Vec<u8>,
    /// The assembled multistream packet.
    buf_out: Vec<u8>,
    /// Float conversion scratch for [`encode_s16`](OpusMSEncoder::encode_s16),
    /// kept for the same reason [`OpusEncoder`] keeps its own.
    buf_from_s16: Vec<f32>,
    /// Reused to apply the self-delimited framing to every stream but the last.
    framer: Repacketizer,
}

/// Shows the layout and per-stream settings, not the encoders' coding state.
impl std::fmt::Debug for OpusMSEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusMSEncoder")
            .field("sample_rate", &self.sample_rate)
            .field("bitrate_bps", &self.bitrate_bps)
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl OpusMSEncoder {
    /// Create a surround encoder for `channels` channels.
    ///
    /// `mapping_family` selects the layout: 0 for mono or stereo, 1 for the
    /// Vorbis surround orders (quad, 5.0, 5.1, 6.1, 7.1) up to 8 channels.
    /// Family 255 codes 1..255 discrete channels as independent mono streams.
    /// The same value belongs in the [`OpusHead`](crate::OpusHead) written
    /// alongside. Other families or channel counts outside their bounds are
    /// [`Error::InvalidArgument`];
    /// [`ChannelLayout::surround`] is where that decision is made.
    pub fn new(
        sample_rate: i32,
        channels: usize,
        mapping_family: u8,
        application: Application,
    ) -> Result<Self> {
        let layout = ChannelLayout::surround(channels, mapping_family)?;
        let mut encoders = Vec::with_capacity(layout.nb_streams);
        for s in 0..layout.nb_streams {
            let ch = if s < layout.nb_coupled_streams { 2 } else { 1 };
            encoders.push(OpusEncoder::new(sample_rate, ch, application)?);
        }
        let mut enc = OpusMSEncoder {
            layout,
            encoders,
            sample_rate,
            bitrate_bps: 64000 * channels as i32,
            buf_stream: Vec::new(),
            buf_packet: Vec::new(),
            buf_out: Vec::new(),
            buf_from_s16: Vec::new(),
            framer: Repacketizer::new(),
        };
        enc.set_bitrate(enc.bitrate_bps);
        Ok(enc)
    }

    /// Split the total bitrate across streams (each coupled stream gets 2x a
    /// mono stream's share, matching its 2 channels).
    pub fn set_bitrate(&mut self, total: i32) {
        self.bitrate_bps = total;
        let units = self.layout.nb_coupled_streams * 2
            + (self.layout.nb_streams - self.layout.nb_coupled_streams);
        let per_unit = if units > 0 {
            total / units as i32
        } else {
            total
        };
        for (s, e) in self.encoders.iter_mut().enumerate() {
            e.bitrate_bps = if s < self.layout.nb_coupled_streams {
                per_unit * 2
            } else {
                per_unit
            };
        }
    }

    /// Opus streams this encoder writes into each packet. Needed for the
    /// [`OpusHead`](crate::OpusHead) that describes the stream.
    pub fn nb_streams(&self) -> usize {
        self.layout.nb_streams
    }

    /// The channel layout this encoder codes to.
    ///
    /// This is what an [`OpusHead`](crate::OpusHead) for the stream has to
    /// declare, and [`OpusHead::for_ms_encoder`](crate::OpusHead::for_ms_encoder)
    /// takes it from here rather than making you derive it a second time.
    pub fn layout(&self) -> &ChannelLayout {
        &self.layout
    }

    /// Total target bitrate across all streams. Set it with
    /// [`set_bitrate`](Self::set_bitrate).
    pub fn bitrate_bps(&self) -> i32 {
        self.bitrate_bps
    }

    /// The per-stream encoders, so every [`OpusEncoder`] setting is reachable.
    ///
    /// A multistream encoder is N ordinary encoders, and rather than mirror ten
    /// settings here — a list that would go stale the moment `OpusEncoder`
    /// gained an eleventh — this hands them over. Set the same thing on all of
    /// them:
    ///
    /// ```
    /// # use fvid_opus::{Application, OpusMSEncoder};
    /// # let mut enc = OpusMSEncoder::new(48_000, 6, 1, Application::Audio)?;
    /// for e in enc.streams_mut() {
    ///     e.use_inband_fec = true;
    ///     e.packet_loss_perc = 10;
    /// }
    /// # Ok::<(), fvid_opus::Error>(())
    /// ```
    ///
    /// [`bitrate_bps`](OpusEncoder::bitrate_bps) is the exception: it is split
    /// across streams from a single total, so set it with
    /// [`set_bitrate`](Self::set_bitrate) and let the split happen.
    pub fn streams_mut(&mut self) -> &mut [OpusEncoder] {
        &mut self.encoders
    }

    /// The per-stream encoders, for reading. See
    /// [`streams_mut`](Self::streams_mut) to change their settings.
    pub fn streams(&self) -> &[OpusEncoder] {
        &self.encoders
    }

    /// Encode one frame of interleaved `input` into a multistream packet,
    /// returning how many bytes of `output` it filled.
    ///
    /// `input` holds `frame_size * nb_channels` samples. Unlike
    /// [`OpusEncoder::encode`], `output.len()` is a plain capacity here and not
    /// a byte budget: a buffer too small is
    /// [`Error::BufferTooSmall`], whose `needed`
    /// tells you exactly how big to make it, rather than a quietly smaller
    /// packet. Each stream is coded to its own share of
    /// [`bitrate_bps`](Self::bitrate_bps).
    pub fn encode(&mut self, input: &[f32], frame_size: usize, output: &mut [u8]) -> Result<usize> {
        self.encode_native(input, frame_size, output, MAX_ENCODING_DEPTH)
    }

    /// [`encode`](Self::encode) from 16-bit PCM.
    ///
    /// Every stream is told its input came from 16 bits, exactly as
    /// [`OpusEncoder::encode_s16`] does for a single stream, and for the same
    /// reason: the digital-silence floor sits at the source's own precision.
    pub fn encode_s16(
        &mut self,
        input: &[i16],
        frame_size: usize,
        output: &mut [u8],
    ) -> Result<usize> {
        let wanted = frame_size * self.layout.nb_channels;
        if input.len() < wanted {
            return Err(Error::InvalidArgument(
                "input is shorter than frame_size * channels",
            ));
        }
        // Converted through a member rather than a fresh `Vec` per call, the
        // way `OpusEncoder::encode_s16` does it.
        let mut converted = std::mem::take(&mut self.buf_from_s16);
        converted.clear();
        converted.extend(input[..wanted].iter().copied().map(i16_to_float));
        let r = self.encode_native(&converted, frame_size, output, 16);
        self.buf_from_s16 = converted;
        r
    }

    fn encode_native(
        &mut self,
        input: &[f32],
        frame_size: usize,
        output: &mut [u8],
        api_lsb_depth: i32,
    ) -> Result<usize> {
        let nch = self.layout.nb_channels;
        if input.len() < frame_size * nch {
            return Err(Error::InvalidArgument(
                "input is shorter than frame_size * channels",
            ));
        }

        // Taken out of `self` so the per-stream encoder below can borrow `self`
        // mutably at the same time; put back before every return.
        let mut stream_buf = std::mem::take(&mut self.buf_stream);
        let mut pkt = std::mem::take(&mut self.buf_packet);
        let mut out = std::mem::take(&mut self.buf_out);
        let mut framer = std::mem::take(&mut self.framer);
        stream_buf.clear();
        stream_buf.resize(frame_size * 2, 0.0);
        pkt.clear();
        pkt.resize(1500 + frame_size, 0);
        out.clear();

        let result = (|| -> Result<usize> {
            for s in 0..self.layout.nb_streams {
                let coupled = s < self.layout.nb_coupled_streams;
                let sch = if coupled { 2 } else { 1 };
                // Gather this stream's channels from the interleaved input.
                if coupled {
                    let l = self
                        .layout
                        .channels_for(ChannelLayout::left_target(s))
                        .next();
                    let r = self
                        .layout
                        .channels_for(ChannelLayout::right_target(s))
                        .next();
                    for i in 0..frame_size {
                        stream_buf[i * 2] = l.map_or(0.0, |c| input[i * nch + c]);
                        stream_buf[i * 2 + 1] = r.map_or(0.0, |c| input[i * nch + c]);
                    }
                } else {
                    let m = self.layout.channels_for(self.layout.mono_target(s)).next();
                    for i in 0..frame_size {
                        stream_buf[i] = m.map_or(0.0, |c| input[i * nch + c]);
                    }
                }
                let n = self.encoders[s].encode_native(
                    &stream_buf[..frame_size * sch],
                    frame_size,
                    &mut pkt,
                    api_lsb_depth,
                )?;
                // All streams but the last are self-delimited so the decoder can
                // find each stream's boundary.
                if s != self.layout.nb_streams - 1 {
                    framer.clear();
                    framer.cat(&pkt[..n])?;
                    framer.out_self_delimited_into(&mut out)?;
                } else {
                    out.extend_from_slice(&pkt[..n]);
                }
            }
            if output.len() < out.len() {
                return Err(Error::buffer_too_small(out.len(), output.len()));
            }
            output[..out.len()].copy_from_slice(&out);
            Ok(out.len())
        })();

        self.buf_stream = stream_buf;
        self.buf_packet = pkt;
        self.buf_out = out;
        self.framer = framer;
        result
    }

    /// The sample rate this encoder was created with.
    pub fn sample_rate(&self) -> i32 {
        self.sample_rate
    }

    /// The number of output channels this encoder codes.
    pub fn channels(&self) -> usize {
        self.layout.nb_channels
    }

    /// Discard every stream's coding state, keeping all settings, as
    /// [`OpusEncoder::reset_state`] does for one.
    pub fn reset_state(&mut self) -> Result<()> {
        for e in &mut self.encoders {
            e.reset_state()?;
        }
        Ok(())
    }
}

/// Multistream decoder: decode each stream and remux to the output channels.
pub struct OpusMSDecoder {
    layout: ChannelLayout,
    decoders: Vec<OpusDecoder>,
    /// One stream's decoded channels, before they are remuxed to the output.
    buf_stream: Vec<f32>,
    /// Float scratch [`decode_s16`](OpusMSDecoder::decode_s16) decodes into
    /// before converting, kept rather than allocated per call.
    buf_f32: Vec<f32>,
    /// Scratch buffer for un-delimiting multistream packets.
    buf_rebuilt: Vec<u8>,
}

/// Shows the layout, not the decoders' coding state.
impl std::fmt::Debug for OpusMSDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusMSDecoder")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl OpusMSDecoder {
    /// Create a surround decoder for `channels` channels.
    ///
    /// `mapping_family` selects the layout, as in
    /// [`OpusMSEncoder::new`]; for a stream read from a file it is the one
    /// [`OpusHead::mapping_family`](crate::OpusHead::mapping_family) carries.
    pub fn new(sample_rate: i32, channels: usize, mapping_family: u8) -> Result<Self> {
        Self::with_layout(
            sample_rate,
            ChannelLayout::surround(channels, mapping_family)?,
        )
    }

    /// Decode an explicit channel map, including duplicate and silent outputs.
    /// Speaker semantics belong to the container's mapping family.
    pub fn with_layout(sample_rate: i32, layout: ChannelLayout) -> Result<Self> {
        layout.validate()?;
        let mut decoders = Vec::with_capacity(layout.nb_streams);
        for s in 0..layout.nb_streams {
            let ch = if s < layout.nb_coupled_streams { 2 } else { 1 };
            decoders.push(OpusDecoder::new(sample_rate, ch)?);
        }
        Ok(OpusMSDecoder {
            layout,
            decoders,
            buf_stream: Vec::new(),
            buf_f32: Vec::new(),
            buf_rebuilt: Vec::new(),
        })
    }

    /// Decode a multistream packet into interleaved `output` (nb_channels per
    /// sample). Returns the number of samples per channel.
    /// Decode one multistream packet into interleaved float PCM.
    ///
    /// As with [`OpusDecoder::decode`], the output is not bounded by ±1.
    pub fn decode(
        &mut self,
        packet: &[u8],
        frame_size: usize,
        output: &mut [f32],
    ) -> Result<usize> {
        self.decode_native(packet, frame_size, output, false)
    }

    /// Decode one multistream packet into interleaved 16-bit PCM.
    ///
    /// Each stream is soft-clipped on its own before the channels are remuxed,
    /// which is where libopus applies it too (`opus_multistream_decode_native`
    /// hands `soft_clip` to each stream's decoder). Clipping the remuxed result
    /// instead would let one stream's peak bend another's audio.
    pub fn decode_s16(
        &mut self,
        packet: &[u8],
        frame_size: usize,
        output: &mut [i16],
    ) -> Result<usize> {
        let nch = self.layout.nb_channels;
        let capacity = frame_size * nch;
        if output.len() < capacity {
            return Err(Error::buffer_too_small(capacity, output.len()));
        }
        let mut pcm = std::mem::take(&mut self.buf_f32);
        pcm.clear();
        pcm.resize(capacity, 0.0);
        let result = self.decode_native(packet, frame_size, &mut pcm, true);
        if let Ok(produced) = result {
            let n = produced * nch;
            for (o, &s) in output[..n].iter_mut().zip(&pcm[..n]) {
                *o = float_to_i16(s);
            }
        }
        self.buf_f32 = pcm;
        result
    }

    fn decode_native(
        &mut self,
        packet: &[u8],
        frame_size: usize,
        output: &mut [f32],
        soft_clip: bool,
    ) -> Result<usize> {
        let nch = self.layout.nb_channels;
        // Every remux below writes `frame_size * nch` samples. libopus takes the
        // buffer on trust because C hands it a bare pointer; here the slice
        // knows its length, so a short one is an error rather than a panic.
        if output.len() < frame_size * nch {
            return Err(Error::buffer_too_small(frame_size * nch, output.len()));
        }
        // Taken out of `self` so a stream's decoder can borrow `self` mutably
        // at the same time; put back before the return.
        let mut buf = std::mem::take(&mut self.buf_stream);
        let mut rebuilt = std::mem::take(&mut self.buf_rebuilt);
        buf.clear();
        buf.resize(frame_size * 2, 0.0);
        let mut data = packet;
        let mut produced = frame_size;

        let result = (|| -> Result<usize> {
            for s in 0..self.layout.nb_streams {
                let coupled = s < self.layout.nb_coupled_streams;
                let last = s == self.layout.nb_streams - 1;
                // The last stream is a normal packet; every earlier one is
                // self-delimited and must have its length prefix stripped before the
                // single-stream decoder, which does not understand that framing.
                let (stream_slice, advance) = if last {
                    (data, data.len())
                } else {
                    let off = take_self_delimited_into(data, &mut rebuilt)?;
                    (rebuilt.as_slice(), off)
                };
                let n = self.decoders[s].decode_native(
                    stream_slice,
                    frame_size,
                    &mut buf,
                    soft_clip,
                )?;
                if s > 0 && n != produced {
                    return Err(Error::InvalidPacket(
                        "multistream packet durations disagree",
                    ));
                }
                produced = n;
                // Remux this stream's channel(s) to the output.
                if coupled {
                    for chan in self.layout.channels_for(ChannelLayout::left_target(s)) {
                        for i in 0..n {
                            output[i * nch + chan] = buf[i * 2];
                        }
                    }
                    for chan in self.layout.channels_for(ChannelLayout::right_target(s)) {
                        for i in 0..n {
                            output[i * nch + chan] = buf[i * 2 + 1];
                        }
                    }
                } else {
                    for chan in self.layout.channels_for(self.layout.mono_target(s)) {
                        for i in 0..n {
                            output[i * nch + chan] = buf[i];
                        }
                    }
                }
                if !last {
                    data = &data[advance..];
                }
            }
            // Unmapped channels (mapping == 255) are silenced.
            for c in 0..nch {
                if self.layout.mapping.get(c).copied() == Some(255) {
                    for i in 0..produced {
                        output[i * nch + c] = 0.0;
                    }
                }
            }
            Ok(produced)
        })();
        self.buf_stream = buf;
        self.buf_rebuilt = rebuilt;
        result
    }

    /// The channel layout this decoder produces.
    pub fn layout(&self) -> &ChannelLayout {
        &self.layout
    }

    /// Opus streams this decoder expects in each packet.
    pub fn nb_streams(&self) -> usize {
        self.layout.nb_streams
    }

    /// The number of output channels this decoder produces.
    pub fn channels(&self) -> usize {
        self.layout.nb_channels
    }

    /// The sample rate this decoder was created with, in Hz.
    pub fn sample_rate(&self) -> i32 {
        self.decoders[0].sample_rate()
    }

    /// The per-stream decoders, so every [`OpusDecoder`] setting is reachable.
    ///
    /// The counterpart of [`OpusMSEncoder::streams_mut`], and the only route to
    /// [`gain_q8`](OpusDecoder::gain_q8) — which is not a nicety. RFC 7845 §5.1
    /// says a player SHOULD apply the gain a file declares, and it says nothing
    /// about mapping family, so a surround stream carrying a non-zero
    /// [`OpusHead::output_gain_q8`](crate::OpusHead::output_gain_q8) plays at
    /// the wrong level with no other symptom. A multistream decoder is N
    /// ordinary decoders and the gain belongs on every one of them:
    ///
    /// ```
    /// # use fvid_opus::{OpusHead, OpusMSDecoder};
    /// # let head = OpusHead::for_layout(
    /// #     &fvid_opus::ChannelLayout::surround(6, 1)?, 48_000);
    /// let mut dec = OpusMSDecoder::new(48_000, 6, head.mapping_family)?;
    /// for d in dec.streams_mut() {
    ///     d.gain_q8 = head.output_gain_q8 as i32;
    /// }
    /// # Ok::<(), fvid_opus::Error>(())
    /// ```
    ///
    /// For mono and stereo, [`OpusHead::decoder`](crate::OpusHead::decoder)
    /// does this for you and there is nothing to remember.
    pub fn streams_mut(&mut self) -> &mut [OpusDecoder] {
        &mut self.decoders
    }

    /// The per-stream decoders, for reading. See
    /// [`streams_mut`](Self::streams_mut) to change their settings.
    ///
    /// [`final_range`](OpusDecoder::final_range) per stream is what a
    /// conformance check compares against a reference decoder's.
    pub fn streams(&self) -> &[OpusDecoder] {
        &self.decoders
    }

    /// Discard every stream's coding state, as [`OpusDecoder::reset_state`]
    /// does for one.
    pub fn reset_state(&mut self) -> Result<()> {
        for d in &mut self.decoders {
            d.reset_state()?;
        }
        Ok(())
    }
}

/// Bandwidth passthrough helper (so callers can cap all streams at once).
impl OpusMSEncoder {
    /// Cap the audio bandwidth of every stream at once, as
    /// [`OpusEncoder::max_bandwidth`] does for one.
    pub fn set_max_bandwidth(&mut self, bw: Bandwidth) {
        for e in &mut self.encoders {
            e.max_bandwidth = bw;
        }
    }
}

#[cfg(test)]
mod packet_duration_tests {
    use super::*;
    #[test]
    fn stream_duration_validation_obeys_boundaries_and_rejects_disagreement() {
        assert_eq!(
            multistream_packet_samples(&[0xf8, 1, 0, 0xf8, 0], 2).unwrap(),
            960
        );
        assert!(multistream_packet_samples(&[0xf8, 1, 0, 0xf0, 0], 2).is_err());
        assert!(multistream_packet_samples(&[0xf8, 255], 2).is_err());
        assert!(multistream_packet_samples(&[], 0).is_err());
        let mut padded = vec![0xfb, 65, 7, 1, 0];
        padded.extend([0; 7]);
        padded.extend([0xf8, 0]);
        assert_eq!(multistream_packet_samples(&padded, 2).unwrap(), 960);
    }
}

#[cfg(test)]
mod explicit_layout_acceptance {
    use super::*;

    #[test]
    fn synthetic_surround_packets_remap_duplicate_and_silent_channels() {
        let packets = include_bytes!(
            "../../../tests/fixtures/playback-errors/opus-surround-native-packets.bin"
        );
        let standard = ChannelLayout::surround(6, 1).unwrap();
        let mut reference = OpusMSDecoder::new(48000, 6, 1).unwrap();
        let mapping = vec![5, 0, 0, 255, 2, 1];
        let mut decoder = OpusMSDecoder::with_layout(
            48000,
            ChannelLayout {
                mapping_family: 255,
                nb_channels: 6,
                nb_streams: standard.nb_streams,
                nb_coupled_streams: standard.nb_coupled_streams,
                mapping: mapping.clone(),
            },
        )
        .unwrap();
        let mut cursor = packets.as_slice();
        let mut count = 0;
        while !cursor.is_empty() {
            let length = u32::from_le_bytes(cursor[..4].try_into().unwrap()) as usize;
            let packet = &cursor[4..4 + length];
            let mut expected = vec![0.0; 5760 * 6];
            let mut actual = vec![123.0; 5760 * 6];
            let n = reference.decode(packet, 5760, &mut expected).unwrap();
            assert_eq!(decoder.decode(packet, 5760, &mut actual).unwrap(), n);
            for (a, b) in actual[..n * 6]
                .chunks_exact(6)
                .zip(expected[..n * 6].chunks_exact(6))
            {
                for (channel, &target) in mapping.iter().enumerate() {
                    let value = if target == 255 {
                        0.0
                    } else {
                        b[standard.mapping.iter().position(|&v| v == target).unwrap()]
                    };
                    assert_eq!(a[channel], value, "packet={count} channel={channel}");
                }
            }
            cursor = &cursor[4 + length..];
            count += 1;
        }
        assert_eq!(count, 6);
    }

    #[test]
    fn explicit_layout_rejects_invalid_geometry_before_decoder_creation() {
        let base = ChannelLayout::surround(6, 1).unwrap();
        for case in 0..7 {
            let mut layout = base.clone();
            match case {
                0 => layout.nb_channels = 0,
                1 => layout.nb_channels = 256,
                2 => layout.nb_streams = 0,
                3 => layout.nb_streams = 256,
                4 => layout.nb_coupled_streams = layout.nb_streams + 1,
                5 => {
                    layout.mapping.pop();
                }
                _ => layout.mapping[0] = 6,
            }
            assert!(
                OpusMSDecoder::with_layout(48000, layout).is_err(),
                "case={case}"
            );
        }
    }
}

#[cfg(test)]
mod discrete_encoding_acceptance {
    use super::*;
    #[test]
    fn discrete_encoder_keeps_every_stream_independent_and_self_delimited() {
        let raw = include_bytes!(
            "../../../tests/fixtures/playback-errors/opus-discrete-encoder-source.f32"
        );
        let source: Vec<f32> = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        for channels in [1usize, 9, 255] {
            let mut encoder = OpusMSEncoder::new(48000, channels, 255, Application::Audio).unwrap();
            let mut references: Vec<_> = (0..3)
                .map(|_| OpusEncoder::new(48000, 1, Application::Audio).unwrap())
                .collect();
            let mut reference_decoders: Vec<_> = (0..3)
                .map(|_| OpusDecoder::new(48000, 1).unwrap())
                .collect();
            let mut decoder = OpusMSDecoder::new(48000, channels, 255).unwrap();
            assert_eq!(encoder.nb_streams(), channels);
            let head = crate::OpusHead::for_ms_encoder(&encoder, 48000);
            assert_eq!(head.mapping_family, 255);
            assert_eq!(
                head.channel_mapping,
                (0..channels).map(|c| c as u8).collect::<Vec<_>>()
            );
            let mut mux = crate::OggOpusWriter::new(Vec::new(), head.clone()).unwrap();
            let mut saved_packets = Vec::new();
            for frame in source.chunks_exact(960) {
                let input: Vec<_> = frame
                    .iter()
                    .flat_map(|v| (0..channels).map(move |c| *v * (c % 3 + 1) as f32 / 3.0))
                    .collect();
                let mut packet = vec![0; channels * 1500];
                let n = encoder.encode(&input, 960, &mut packet).unwrap();
                packet.truncate(n);
                let mut expected_packets = Vec::new();
                let mut expected_pcm = Vec::new();
                for group in 0..3 {
                    let samples: Vec<_> = frame
                        .iter()
                        .map(|v| *v * (group + 1) as f32 / 3.0)
                        .collect();
                    let mut expected = vec![0; 1500];
                    let count = references[group]
                        .encode(&samples, 960, &mut expected)
                        .unwrap();
                    expected.truncate(count);
                    let mut pcm = vec![0.0; 960];
                    assert_eq!(
                        reference_decoders[group]
                            .decode(&expected, 960, &mut pcm)
                            .unwrap(),
                        960
                    );
                    expected_packets.push(expected);
                    expected_pcm.push(pcm);
                }
                let mut cursor = packet.as_slice();
                let mut rebuilt = Vec::new();
                for stream in 0..channels {
                    let expected = &expected_packets[stream % 3];
                    if stream + 1 == channels {
                        assert_eq!(cursor, expected);
                    } else {
                        let used = take_self_delimited_into(cursor, &mut rebuilt).unwrap();
                        assert_eq!(&rebuilt, expected);
                        cursor = &cursor[used..];
                    }
                }
                mux.write_packet(&packet).unwrap();
                saved_packets.push(packet.clone());
                assert_eq!(multistream_packet_samples(&packet, channels).unwrap(), 960);
                let mut pcm = vec![0.0; 960 * channels];
                assert_eq!(decoder.decode(&packet, 960, &mut pcm).unwrap(), 960);
                for (sample, row) in pcm.chunks_exact(channels).enumerate() {
                    for (channel, value) in row.iter().enumerate() {
                        assert_eq!(*value, expected_pcm[channel % 3][sample]);
                    }
                }
            }
            assert_eq!(mux.granule(), 2880);
            let encoded = mux.finish().unwrap();
            let mut reader = crate::OggOpusReader::new(std::io::Cursor::new(encoded)).unwrap();
            assert_eq!(reader.head().channel_mapping, head.channel_mapping);
            assert_eq!(reader.head().stream_count, head.stream_count);
            for (index, expected) in saved_packets.iter().enumerate() {
                let packet = reader.read_packet().unwrap().unwrap();
                assert_eq!(&packet.data, expected);
                if index == 2 {
                    assert_eq!(packet.page_granule, 2880);
                    assert!(packet.end_of_stream);
                }
            }
            assert!(reader.read_packet().unwrap().is_none());
        }
    }
}
