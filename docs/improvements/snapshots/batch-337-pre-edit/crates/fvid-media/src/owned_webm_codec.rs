//! Owned WebM packet-to-planar adapter for the shared streaming pipeline.
#[path = "owned_webm_presentation.rs"]
mod presentation;
pub(crate) use presentation::Presentation;
use crate::owned_frame::{GeometryFrame, buffer};
use fvid_codecs::{
    Error,
    codec::{av1_decoder, avc_decoder, hevc_decoder, vp9, vp9_decoder},
};
type Result<T> = std::result::Result<T, String>;
pub(crate) const UNSUPPORTED_PREFIX: &str = "unsupported owned video capability: ";
fn core_error(error: Error) -> String {
    match error {
        Error::Unsupported(value) => format!("{UNSUPPORTED_PREFIX}{value}"),
        other => other.to_string(),
    }
}
pub(crate) fn video_track<R: std::io::Read+std::io::Seek>(reader:&crate::owned_webm::WebmReader<R>) -> Option<&crate::owned_webm::Track> {
    let candidates:Vec<_>=reader.tracks.iter().filter(|t|t.kind==1 && matches!(t.codec.as_str(),
        "V_FFV1"|"V_VP9"|"V_AV1"|"V_MPEG4/ISO/AVC"|"V_MPEGH/ISO/HEVC")).map(|t|t.number).collect();
    let number=reader.preferred_track_numbers(&candidates).first().copied()?;
    reader.tracks.iter().find(|t|t.number==number)
}
pub(crate) struct Decoded {
    pub frame: GeometryFrame,
    pub depth: u8,
    pub alpha: Option<Vec<u8>>,
}
enum Kind {
    Ffv1(crate::owned_ffv1_decoder::Decoder),
    Vp9(vp9_decoder::Decoder),
    Av1(av1_decoder::Decoder),
    Hevc(hevc_decoder::HevcDecoder),
    Avc(avc_decoder::AvcDecoder),
}
pub(crate) struct Decoder {
    kind: Kind,
    mono: Option<bool>,
    colour: Option<(bool, u8, u8, u8)>,
}
impl Decoder {
    pub fn new(codec: &str, private: &[u8], width: usize, height: usize) -> Result<Option<Self>> {
        let kind = match codec {
            "V_FFV1" => Kind::Ffv1(crate::owned_ffv1_decoder::Decoder::from_configuration(
                width,
                height,
                private,
                usize::MAX,
            )?),
            "V_MPEG4/ISO/AVC" => {
                let decoder = match avc_decoder::AvcDecoder::new(private, usize::MAX) {
                    Ok(value) => value,
                    Err(Error::Unsupported(_)) => return Ok(None),
                    Err(error) => return Err(error.to_string()),
                };
                Kind::Avc(decoder)
            }
            "V_MPEGH/ISO/HEVC" => {
                let decoder = match hevc_decoder::HevcDecoder::from_configuration(private, usize::MAX) {
                    Ok(value) => value,
                    Err(Error::Unsupported(_)) => return Ok(None),
                    Err(error) => return Err(error.to_string()),
                };
                Kind::Hevc(decoder)
            }
            "V_VP9" => Kind::Vp9(vp9_decoder::Decoder::new(usize::MAX)),
            "V_AV1" => {
                let decoder = match av1_decoder::Decoder::from_configuration(private, usize::MAX) {
                    Ok(value) => value,
                    Err(Error::Unsupported(_)) => return Ok(None),
                    Err(error) => return Err(error.to_string()),
                };
                Kind::Av1(decoder)
            }
            _ => return Ok(None),
        };
        Ok(Some(Self {
            kind,
            mono: None,
            colour: None,
        }))
    }
    pub(crate) fn has_pending_field(&self) -> bool {
        matches!(&self.kind, Kind::Avc(d) if d.has_pending_field())
    }
    pub(crate) fn output_is_field_pair(&self) -> bool {
        matches!(&self.kind, Kind::Avc(d) if d.output_is_field_pair())
    }
    pub fn monochrome(&self) -> Option<bool> {
        self.mono
    }
    pub fn colour(&self) -> Option<(bool, u8, u8, u8)> {
        self.colour
    }
    pub fn hdr(&self) -> Option<crate::owned_matroska::HdrMetadata> {
        match &self.kind {
            Kind::Av1(decoder) => Some(hdr_metadata(decoder.hdr())),
            Kind::Hevc(decoder) => Some(hdr_metadata(decoder.hdr())),
            _ => None,
        }
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Decoded>> {
        let mut shown = None;
        match &mut self.kind {
            Kind::Ffv1(decoder) => {
                let decoded = decoder.decode(packet)?;
                self.mono = decoder.monochrome();
                return Ok(Some(Decoded {
                    alpha: decoded.alpha,
                    frame: decoded.frame,
                    depth: decoded.depth,
                }));
            }
            Kind::Avc(decoder) => {
                let picture = decoder.decode_order(packet).map_err(core_error)?;
                if let Some(picture) = picture {
                    let (width, height) = picture.dimensions();
                    let chroma = picture.chroma_format().map_err(core_error)?;
                    let mono = chroma == 0;
                    self.mono = Some(mono);
                    self.colour = decoder
                        .active_vui()
                        .or_else(|| decoder.recorded_vui())
                        .and_then(|v| v.video_signal)
                        .map(|(_, full, codes)| {
                            let [primaries, transfer, matrix] = codes.unwrap_or([2, 2, 2]);
                            (full, matrix, primaries, transfer)
                        });
                    let rgb = self.colour.is_some_and(|c| c.1 == 0);
                    if rgb && chroma != 3 {
                        return Err("AVC identity RGB requires 4:4:4".into());
                    }
                    let sub = match chroma {
                        0 | 3 => [1, 1],
                        1 => [2, 2],
                        2 => [2, 1],
                        _ => return Err("invalid AVC chroma format".into()),
                    };
                    shown = Some(Decoded {
                        frame: GeometryFrame {
                            width,
                            height,
                            subsampling: if rgb { None } else { Some(sub) },
                            data: crate::owned_mp4_video_decode::pack_avc_format(&picture, rgb)?,
                        },
                        depth: if mono {
                            picture.bit_depth
                        } else {
                            picture.bit_depth.max(picture.chroma_bit_depth)
                        },
                        alpha: None,
                    });
                }
            }
            Kind::Hevc(decoder) => {
                if let Some(decoded) = decoder.decode_packet(packet).map_err(core_error)?.filter(|f| f.output) {
                    let p = &decoded.picture;
                    let sps = decoder.parameters().0;
                    let mono = sps.chroma_format == 0;
                    let sub = match sps.chroma_format {
                        0 | 3 => [false, false], 1 => [true, true], 2 => [true, false],
                        _ => return Err("invalid HEVC chroma format".into()),
                    };
                    self.colour = sps.vui.as_ref().and_then(|v| v.signal.as_ref()).map(|signal| {
                        let [primaries, transfer, matrix] = signal.colour.unwrap_or([2, 2, 2]);
                        (signal.full_range, matrix, primaries, transfer)
                    });
                    self.mono = Some(mono);
                    let rgb = self.colour.is_some_and(|c| c.1 == 0);
                    if rgb && sps.chroma_format != 3 { return Err("HEVC identity RGB requires 4:4:4".into()); }
                    let data = crate::owned_mp4_video_decode::pack_hevc(p, sub, mono, rgb)?;
                    let width = p.dimensions[0].checked_sub(p.crop[0]).and_then(|v|v.checked_sub(p.crop[1])).ok_or("HEVC crop exceeds width")? as usize;
                    let height = p.dimensions[1].checked_sub(p.crop[2]).and_then(|v|v.checked_sub(p.crop[3])).ok_or("HEVC crop exceeds height")? as usize;
                    shown = Some(Decoded {
                        frame: GeometryFrame { width, height, subsampling: if rgb { None } else { Some(sub.map(|s|if s {2}else{1})) }, data },
                        depth: if mono {p.depth[0]} else {p.depth[0].max(p.depth[1])}, alpha: None,
                    });
                }
            }
            Kind::Vp9(decoder) => {
                for frame in vp9::frames(packet).map_err(core_error)? {
                    let decoded = decoder.decode(frame).map_err(core_error)?;
                    if !decoded.header.show_frame {
                        continue;
                    }
                    if shown.is_some() {
                        return Err(
                            "multiple visible frames in one WebM packet need distinct timestamps"
                                .into(),
                        );
                    }
                    let format = decoded.header.picture.format;
                    if format.color_space == 7 {
                        self.colour = Some((true, 0, 1, 13));
                        self.mono = Some(false);
                        let p = &decoded.picture;
                        shown = Some(Decoded { alpha: None, frame: crate::owned_frame::pack_gbr(p.size, p.depth,
                            p.planes.each_ref().map(|plane| (plane.width, plane.height, plane.samples.as_slice())))?, depth: p.depth });
                        continue;
                    }
                    let matrix = match format.color_space {
                        2 => 1,
                        4 => 7,
                        5 => 9,
                        _ => 6,
                    };
                    self.colour = Some((format.full_range, matrix, 0, 0));
                    self.mono = Some(false);
                    let picture = &decoded.picture;
                    let planes = picture
                        .planes
                        .each_ref()
                        .map(|plane| (plane.width, plane.height, plane.samples.as_slice()));
                    shown = Some(pack(
                        picture.size,
                        picture.depth,
                        planes,
                        format.subsampling,
                        false,
                    )?);
                }
            }
            Kind::Av1(decoder) => {
                if let Some(decoded) = decoder
                    .decode_container_sample(packet)
                    .map_err(core_error)?
                {
                    if decoded.color.monochrome && !decoded.color.full_range {
                        return Err(format!(
                            "{UNSUPPORTED_PREFIX}studio-range monochrome streaming transforms"
                        ));
                    }
                    self.colour = Some((
                        decoded.color.full_range,
                        decoded.color.matrix,
                        decoded.color.primaries,
                        decoded.color.transfer,
                    ));
                    self.mono = Some(decoded.color.monochrome);
                    shown = Some(pack_av1(&decoded)?);
                }
            }
        }
        Ok(shown)
    }
}
pub(crate) fn pack_av1(decoded: &av1_decoder::Decoded) -> Result<Decoded> {
    let picture = &decoded.picture;
    if decoded.color.matrix == 0 && !decoded.color.monochrome {
        let [width, height] = picture.size.map(|v| v as usize);
        let bytes = if picture.depth == 8 { 1 } else { 2 };
        let length = width.checked_mul(height).and_then(|n| n.checked_mul(3 * bytes))
            .ok_or("RGB video frame size overflow")?;
        let mut data = buffer(length)?;
        for plane in &picture.planes {
            if plane.width < width || plane.height < height
                || plane.width.checked_mul(plane.height) != Some(plane.samples.len()) {
                return Err("decoded RGB video plane geometry mismatch".into());
            }
        }
        let mut cursor = 0;
        for y in 0..height {
            for x in 0..width {
                // AV1 identity planes carry G, B, R; GeometryFrame carries RGB.
                for channel in [2, 0, 1] {
                    let plane = &picture.planes[channel];
                    let value = plane.samples[y * plane.width + x];
                    if value as u32 >= 1u32 << picture.depth {
                        return Err("RGB video sample exceeds coded depth".into());
                    }
                    if bytes == 1 {
                        data[cursor] = value as u8;
                    } else {
                        data[cursor..cursor + 2].copy_from_slice(&value.to_le_bytes());
                    }
                    cursor += bytes;
                }
            }
        }
        return Ok(Decoded { alpha: None, frame: GeometryFrame { width, height, subsampling: None, data }, depth: picture.depth });
    }
    let planes = picture
        .planes
        .each_ref()
        .map(|plane| (plane.width, plane.height, plane.samples.as_slice()));
    pack(
        picture.size,
        picture.depth,
        planes,
        decoded.color.subsampling,
        decoded.color.monochrome,
    )
}

pub(crate) fn pack(
    size: [u32; 2],
    depth: u8,
    planes: [(usize, usize, &[u16]); 3],
    sub: [bool; 2],
    mono: bool,
) -> Result<Decoded> {
    let [width, height] = size.map(|v| v as usize);
    let subsampling = if mono {
        [1, 1]
    } else {
        sub.map(|v| if v { 2 } else { 1 })
    };
    let samples = width
        .checked_mul(height)
        .ok_or("video plane size overflow")?;
    let chroma = width
        .div_ceil(subsampling[0])
        .checked_mul(height.div_ceil(subsampling[1]))
        .ok_or("video plane size overflow")?;
    let bytes = if depth == 8 { 1 } else { 2 };
    let mut data = buffer(
        samples
            .checked_add(chroma.checked_mul(2).ok_or("video frame size overflow")?)
            .and_then(|v| v.checked_mul(bytes))
            .ok_or("video frame size overflow")?,
    )?;
    let mut cursor = 0;
    for (component, &(pw, ph, values)) in planes.iter().enumerate() {
        let count = if component == 0 { samples } else { chroma };
        let w = if component == 0 {
            width
        } else {
            width.div_ceil(subsampling[0])
        };
        let h = if component == 0 {
            height
        } else {
            height.div_ceil(subsampling[1])
        };
        if !(mono && component != 0)
            && (pw < w || ph < h || pw.checked_mul(ph) != Some(values.len()))
        {
            return Err("decoded video plane geometry mismatch".into());
        }
        debug_assert_eq!(w * h, count);
        for row in 0..h {
            for col in 0..w {
                let value = if mono && component != 0 {
                    1u16 << (depth - 1)
                } else {
                    values[row * pw + col]
                };
                if bytes == 1 {
                    data[cursor] =
                        u8::try_from(value).map_err(|_| "8-bit video sample overflow")?;
                    cursor += 1;
                } else {
                    data[cursor..cursor + 2].copy_from_slice(&value.to_le_bytes());
                    cursor += 2;
                }
            }
        }
    }
    Ok(Decoded {
        alpha: None,
        frame: GeometryFrame {
            width,
            height,
            subsampling: Some(subsampling),
            data,
        },
        depth,
    })
}

/// Infer only missing display durations. Hidden codec pictures never delimit a
/// visible interval. The terminal interval uses declared Segment end, otherwise
/// the preceding visible interval; an isolated untimed picture stays unadmitted.
pub(crate) fn presentation_durations<R: std::io::Read + std::io::Seek>(
    reader: &mut crate::owned_webm::WebmReader<R>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<Option<Vec<Option<u64>>>> {
    let Some(track) = video_track(reader) else {
        return Err("input has no video stream".into());
    };
    let number = track.number;
    let default = track.default_duration_ns;
    if let Some(limit) = options
        .and_then(|o| o.max_controlled_bytes)
        .filter(|&limit| limit > 0)
        && reader
            .packets
            .len()
            .saturating_mul(std::mem::size_of::<Option<u64>>())
            > limit
    {
        return Err("controlled memory budget exceeded: Matroska duration index".into());
    }
    let mut durations = Vec::new();
    durations
        .try_reserve_exact(reader.packets.len())
        .map_err(|_| "duration index allocation failed")?;
    durations.extend(reader.packets.iter().map(|p| {
        p.duration_ns
            .filter(|&v| v != 0)
            .or((default != 0).then_some(default))
    }));
    if reader
        .packets
        .iter()
        .enumerate()
        .all(|(i, p)| p.track != number || durations[i].is_some())
    {
        return Ok(Some(durations));
    }
    let Some(mut decoder) = Decoder::new(
        &track.codec,
        &track.codec_private,
        track.width as usize,
        track.height as usize,
    )?
    else {
        return Ok(None);
    };
    if let Some(limit) = options
        .and_then(|o| o.max_controlled_bytes)
        .filter(|&limit| limit > 0)
        && reader
            .packets
            .len()
            .saturating_mul(
                std::mem::size_of::<Option<u64>>()
                    .saturating_add(std::mem::size_of::<(usize, i64)>()),
            )
            > limit
    {
        return Err("controlled memory budget exceeded: Matroska presentation index".into());
    }
    let mut visible = Vec::new();
    visible
        .try_reserve_exact(reader.packets.len())
        .map_err(|_| "presentation index allocation failed")?;
    let mut consumed = 0u64;
    let mut pending_field = None;
    for index in 0..reader.packets.len() {
        let packet = &reader.packets[index];
        if packet.track != number {
            continue;
        }
        if options
            .and_then(|o| o.cancel.as_ref())
            .is_some_and(fvid_control::CancelFlag::is_cancelled)
        {
            return Err("media operation cancelled".into());
        }
        if options
            .and_then(|o| o.max_packets)
            .is_some_and(|limit| consumed >= limit)
        {
            return Err("FFV1 input packet count exceeds limit".into());
        }
        consumed += 1;
        let (shown, pts) = (!packet.invisible, packet.pts_ns);
        let bytes = reader.read_packet(index).map_err(|e| e.to_string())?;
        if options.is_some_and(|o| bytes.len() > o.max_packet_bytes) {
            return Err("input packet exceeds byte limit".into());
        }
        let decoded = decoder.decode(&bytes);
        if let Ok(Some(ref dec)) = decoded
            && let Some(limit) = options
                .and_then(|o| o.max_controlled_bytes)
                .filter(|&limit| limit > 0)
            && dec
                .frame
                .data
                .len()
                .saturating_add(dec.alpha.as_ref().map_or(0, Vec::len))
                > limit
        {
            return Err("video frame exceeds controlled memory budget".into());
        }
        if decoder.has_pending_field() { pending_field.get_or_insert(index); }
        match decoded {
            Ok(Some(_)) if decoder.output_is_field_pair() => {
                let first = pending_field.take().ok_or("missing AVC first-field timing")?;
                if reader.packets[first].invisible != !shown { return Err("AVC field pair visibility differs".into()); }
                durations[first] = durations[first].zip(durations[index])
                    .map(|(a,b)|a.checked_add(b).ok_or("AVC field-pair duration overflow")).transpose()?;
                if shown { visible.push((first, reader.packets[first].pts_ns)); }
            }
            Ok(Some(_)) if shown => visible.push((index, pts)),
            Ok(_) => {}
            Err(error) if error.starts_with(UNSUPPORTED_PREFIX) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    if pending_field.is_some() { return Err("unpaired AVC field at end of Matroska".into()); }
    visible.sort_unstable_by_key(|&(index, pts)| (pts, index));
    for (position, &(index, pts)) in visible.iter().enumerate() {
        if durations[index].is_some() {
            continue;
        }
        let end = if let Some(&(_, next)) = visible.get(position + 1) {
            next
        } else if let Some(end) = reader
            .duration_ns
            .and_then(|v| i64::try_from(v).ok())
            .filter(|&v| v > pts)
        {
            end
        } else if let Some(&(_, previous)) = position.checked_sub(1).and_then(|p| visible.get(p)) {
            let Some(delta) = pts.checked_sub(previous) else {
                return Ok(None);
            };
            let Some(end) = pts.checked_add(delta) else {
                return Ok(None);
            };
            end
        } else {
            return Ok(None);
        };
        let Some(interval) = end.checked_sub(pts).filter(|&d| d > 0) else {
            return Ok(None);
        };
        durations[index] = Some(interval as u64);
    }
    Ok(Some(durations))
}

pub(crate) fn hdr_metadata(
    hdr: fvid_codecs::color::hdr::HdrMetadata,
) -> crate::owned_matroska::HdrMetadata {
    let corner =
        |value: fvid_codecs::color::primaries::Chromaticity| crate::owned_matroska::Chromaticity {
            x: value.x,
            y: value.y,
        };
    crate::owned_matroska::HdrMetadata {
        mastering: hdr
            .mastering
            .map(|m| crate::owned_matroska::MasteringDisplay {
                red: corner(m.red),
                green: corner(m.green),
                blue: corner(m.blue),
                white: corner(m.white),
                max_luminance: m.max_luminance,
                min_luminance: m.min_luminance,
            }),
        light: crate::owned_matroska::ContentLight {
            max_cll: hdr.light.max_cll,
            max_fall: hdr.light.max_fall,
        },
    }
}

#[cfg(test)]
mod avc_pipeline_tests {
    use super::*;
    #[test]
    fn separate_fields_expose_pair_binding_state() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/avc-field-pcm-8bit-top-first.mp4");
        let mut reader = crate::owned_mp4::Mp4Reader::open(
            std::io::Cursor::new(std::fs::read(path).unwrap()), Default::default()).unwrap();
        let private = reader.tracks()[0].configuration.clone();
        let mut decoder = Decoder::new("V_MPEG4/ISO/AVC", &private, 16, 32).unwrap().unwrap();
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        assert!(decoder.decode(&packet).unwrap().is_none());
        assert!(decoder.has_pending_field());
        assert!(!decoder.output_is_field_pair());
        reader.read_packet(0, 1, &mut packet).unwrap();
        assert!(decoder.decode(&packet).unwrap().is_some());
        assert!(!decoder.has_pending_field());
        assert!(decoder.output_is_field_pair());
    }
}
