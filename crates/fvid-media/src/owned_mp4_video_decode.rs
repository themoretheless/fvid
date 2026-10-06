//! Owned AVC/HEVC packet decoding with shared MP4 presentation edits.
use fvid_codecs::{
    Error,
    codec::{avc_decoder::AvcDecoder, hevc_decoder::HevcDecoder},
};
use fvid_media_info::{DecodeStats, DecodeTransform};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn try_decode(
    source: &Path,
    transform: &DecodeTransform,
) -> Result<Option<DecodeStats>> {
    decode_presented(source, transform, None, None)
}

pub(crate) type Visitor<'a> = dyn FnMut(&FrameMetadata, &[u8], u64, u64) -> Result<()> + 'a;
pub(crate) fn decode_presented(
    source: &Path,
    transform: &DecodeTransform,
    mut visit: Option<&mut Visitor<'_>>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<Option<DecodeStats>> {
    if transform
        .input_format
        .as_deref()
        .is_some_and(|v| !matches!(v, "mp4" | "mov"))
    {
        return Ok(None);
    }
    let mut reader = match crate::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            packet_bytes: options.map_or(32 << 20, |o| o.max_packet_bytes),
            ..Default::default()
        },
    ) {
        Ok(value) => value,
        Err(error) if error.is_unsupported() => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let Some(index) = reader
        .tracks()
        .iter()
        .position(|track| track.handler == *b"vide")
    else {
        return Ok(None);
    };
    let track = &reader.tracks()[index];
    if !matches!(&track.codec, b"avc1" | b"avc3" | b"hvc1" | b"hev1") || track.rotation != 0 {
        return Ok(None);
    }
    let edits = match crate::owned_video_timeline::map_edits(
        track
            .edits
            .iter()
            .map(|edit| (edit.duration, edit.media_time)),
        track.timescale,
        reader.movie_timescale(),
    ) {
        Ok(value) => value,
        Err(Error::Unsupported(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let scale = track.timescale;
    let container_colour = track.colour;
    let container_hdr = track.hdr;
    let mut spool = if visit.is_some() {
        Some(crate::owned_reverse::Reverse::new()?)
    } else {
        None
    };
    let mut spool_count = 0;
    let mut storage_format = None;
    let mut pixels = Vec::new();
    let configuration = track.configuration.clone();
    let samples = track.samples.len();
    let avc = matches!(&track.codec, b"avc1" | b"avc3");
    let mut stats = DecodeStats {
        backend: "owned MP4 compressed video decode",
        video_frames: 0,
        width: 0,
        height: 0,
        pixel_format: String::new(),
        decode_errors: 0,
    };
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(samples)
        .map_err(|_| "MP4 presentation metadata allocation failed")?;
    let mut packet = Vec::new();
    if avc {
        let mut decoder = match AvcDecoder::new(&configuration, usize::MAX) {
            Ok(value) => value,
            Err(Error::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let mut pending_field = None;
        for sample in 0..samples {
            check_options(options, sample)?;
            let mut timing = reader.tracks()[index]
                .samples
                .get(sample)
                .ok_or("missing MP4 sample timing")?;
            reader
                .read_packet(index, sample, &mut packet)
                .map_err(|e| e.to_string())?;
            let decoded = match decoder.decode_order(&packet) {
                Ok(value) => value,
                Err(Error::Unsupported(_)) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            let mut output_sample = sample;
            if decoder.has_pending_field() {
                pending_field.get_or_insert(sample);
            }
            if decoder.output_is_field_pair() {
                output_sample = pending_field
                    .take()
                    .ok_or("missing AVC first-field timing")?;
                let first = reader.tracks()[index]
                    .samples
                    .get(output_sample)
                    .ok_or("missing AVC first-field sample")?;
                timing.pts = first.pts;
                timing.duration = timing
                    .duration
                    .checked_add(first.duration)
                    .ok_or("AVC field-pair duration overflow")?;
            }
            if let Some(picture) = decoded {
                let (width, height) = picture.dimensions();
                if let Some(spool) = spool.as_mut() {
                    let current = (
                        [width as u32, height as u32],
                        picture.bit_depth,
                        [true, true],
                        false,
                    );
                    if storage_format.is_some_and(|previous| previous != current) {
                        return Ok(None);
                    }
                    storage_format = Some(current);
                    pixels = pack_avc(&picture)?;
                    spool.push(&pixels, 0, 0)?;
                }
                let mut colour = container_colour;
                if let Some((_, full, codes)) = decoder.active_vui().and_then(|v| v.video_signal) {
                    colour.full_range = full;
                    if let Some([primaries, transfer, matrix]) = codes {
                        colour.primaries = primaries;
                        colour.transfer = transfer;
                        colour.matrix = matrix;
                    }
                }
                frames.push(FrameMetadata {
                    colour,
                    hdr: container_hdr,
                    slot: spool_count,
                    pts: timing.pts,
                    duration: i64::from(timing.duration),
                    sample: output_sample,
                    size: [
                        u32::try_from(width).map_err(|_| "AVC width overflow")?,
                        u32::try_from(height).map_err(|_| "AVC height overflow")?,
                    ],
                    depth: picture.bit_depth,
                    sub: [true, true],
                    mono: false,
                });
                spool_count += 1;
            }
        }
        if decoder.has_pending_field() {
            return Err("unpaired AVC field at end of MP4".into());
        }
    } else {
        let mut decoder = match HevcDecoder::from_configuration(&configuration, usize::MAX) {
            Ok(value) => value,
            Err(Error::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        for sample in 0..samples {
            check_options(options, sample)?;
            let timing = reader.tracks()[index]
                .samples
                .get(sample)
                .ok_or("missing MP4 sample timing")?;
            reader
                .read_packet(index, sample, &mut packet)
                .map_err(|e| e.to_string())?;
            let decoded = match decoder.decode_packet(&packet) {
                Ok(value) => value,
                Err(Error::Unsupported(_)) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            if let Some(decoded) = decoded.filter(|value| value.output) {
                let picture = &decoded.picture;
                let size = [
                    picture.dimensions[0]
                        .checked_sub(picture.crop[0])
                        .and_then(|v| v.checked_sub(picture.crop[1]))
                        .ok_or("HEVC crop exceeds width")?,
                    picture.dimensions[1]
                        .checked_sub(picture.crop[2])
                        .and_then(|v| v.checked_sub(picture.crop[3]))
                        .ok_or("HEVC crop exceeds height")?,
                ];
                let chroma = decoder.parameters().0.chroma_format;
                let sub = match chroma {
                    0 | 3 => [false, false],
                    1 => [true, true],
                    2 => [true, false],
                    _ => return Err("invalid HEVC chroma format".into()),
                };
                let output_depth = if chroma == 0 {
                    picture.depth[0]
                } else {
                    picture.depth[0].max(picture.depth[1])
                };
                if let Some(spool) = spool.as_mut() {
                    let current = (size, output_depth, sub, chroma == 0);
                    if storage_format.is_some_and(|previous| previous != current) {
                        return Ok(None);
                    }
                    storage_format = Some(current);
                    pixels = pack_hevc(picture, sub, chroma == 0)?;
                    spool.push(&pixels, 0, 0)?;
                }
                let mut colour = container_colour;
                if let Some(signal) = decoder.parameters().0.vui.as_ref().and_then(|v| v.signal) {
                    colour.full_range = signal.full_range;
                    if let Some([primaries, transfer, matrix]) = signal.colour {
                        colour.primaries = primaries;
                        colour.transfer = transfer;
                        colour.matrix = matrix;
                    }
                }
                let mut hdr = crate::owned_webm_codec::hdr_metadata(decoder.hdr());
                hdr.mastering = hdr.mastering.or(container_hdr.mastering);
                if hdr.light.max_cll == 0.0 {
                    hdr.light.max_cll = container_hdr.light.max_cll;
                }
                if hdr.light.max_fall == 0.0 {
                    hdr.light.max_fall = container_hdr.light.max_fall;
                }
                frames.push(FrameMetadata {
                    colour,
                    hdr,
                    slot: spool_count,
                    pts: timing.pts,
                    duration: i64::from(timing.duration),
                    sample,
                    size,
                    depth: output_depth,
                    sub,
                    mono: chroma == 0,
                });
                spool_count += 1;
            }
        }
    }
    normalize_presentations(&mut frames)?;
    if edits.is_empty() {
        for frame in &frames {
            if frame
                .pts
                .checked_add(frame.duration)
                .ok_or("video timestamp overflow")?
                > 0
            {
                emit(
                    frame,
                    frame.pts.max(0),
                    frame
                        .pts
                        .checked_add(frame.duration)
                        .ok_or("video timestamp overflow")?,
                    scale,
                    &mut spool,
                    &mut pixels,
                    &mut visit,
                    &mut stats,
                )?;
            }
        }
    } else {
        for edit in &edits {
            // Normalization validated every endpoint and made ends monotone.
            let first =
                frames.partition_point(|frame| frame.pts + frame.duration <= edit.media_start);
            for frame in &frames[first..] {
                if frame.pts >= edit.media_end {
                    break;
                }
                if crate::owned_video_timeline::appearances(
                    std::slice::from_ref(edit),
                    frame.pts,
                    frame.duration,
                )
                .map_err(|e| e.to_string())?
                    != 0
                {
                    let start = frame.pts.max(edit.media_start);
                    let end = (frame.pts + frame.duration).min(edit.media_end);
                    let mapped = edit
                        .movie_start
                        .checked_add(start - edit.media_start)
                        .ok_or("movie timestamp overflow")?;
                    emit(
                        frame,
                        mapped,
                        mapped
                            .checked_add(end - start)
                            .ok_or("movie timestamp overflow")?,
                        scale,
                        &mut spool,
                        &mut pixels,
                        &mut visit,
                        &mut stats,
                    )?;
                }
            }
        }
    }
    if stats.video_frames == 0 {
        return Err("input has no visible decoded video frames".into());
    }
    Ok(Some(stats))
}

#[derive(Clone, Copy)]
pub(crate) struct FrameMetadata {
    pub colour: crate::owned_matroska::ColourDescription,
    pub hdr: crate::owned_matroska::HdrMetadata,
    slot: u64,
    pts: i64,
    duration: i64,
    sample: usize,
    pub size: [u32; 2],
    pub depth: u8,
    pub sub: [bool; 2],
    pub mono: bool,
}
impl FrameMetadata {
    fn account(&self, stats: &mut DecodeStats) -> Result<()> {
        crate::owned_compressed_video::account(
            stats, self.size, self.depth, self.sub, self.mono, false,
        )
    }
}
// Decode order determines the last equal-PTS picture. Display order determines
// intervals; the terminal equal-PTS group retains its accumulated nominal span.
fn normalize_presentations(frames: &mut Vec<FrameMetadata>) -> Result<()> {
    frames.sort_unstable_by_key(|frame| (frame.pts, frame.sample));
    let mut write = 0;
    for read in 0..frames.len() {
        let mut frame = frames[read];
        if write > 0 && frames[write - 1].pts == frame.pts {
            frame.duration = frame
                .duration
                .checked_add(frames[write - 1].duration)
                .ok_or("duplicate-PTS duration overflow")?;
            frames[write - 1] = frame;
        } else {
            frames[write] = frame;
            write += 1;
        }
    }
    frames.truncate(write);
    for index in 0..frames.len() {
        if index + 1 < frames.len() {
            frames[index].duration = frames[index + 1]
                .pts
                .checked_sub(frames[index].pts)
                .ok_or("presentation duration overflow")?;
        }
        if frames[index].duration <= 0 {
            return Err("invalid video duration".into());
        }
        frames[index]
            .pts
            .checked_add(frames[index].duration)
            .ok_or("video timestamp overflow")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(pts: i64, duration: i64, sample: usize) -> FrameMetadata {
        FrameMetadata {
            colour: Default::default(),
            hdr: Default::default(),
            slot: sample as u64,
            pts,
            duration,
            sample,
            size: [sample as u32 + 1, 1],
            depth: 8,
            sub: [true, true],
            mono: false,
        }
    }
    #[test]
    fn complementary_fields_preserve_first_timing_and_pixels() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for name in [
            "avc-field-pcm-8bit-top-first",
            "avc-field-pcm-10bit-bottom-first-mixed-long",
            "avc-field-skip-8bit-top-first",
            "avc-field-skip-8bit-bottom-first",
            "avc-field-skip-10bit-top-first",
            "avc-field-skip-10bit-bottom-first",
            "avc-field-motion-8bit-top-first-x1-y1",
            "avc-field-motion-10bit-bottom-first-x-7-y6",
            "avc-field-partition-8bit-top-first-type3-sub3",
            "avc-field-partition-10bit-bottom-first-type4-sub2",
            "avc-field-residual-8bit-top-first-all-ac",
            "avc-field-residual-10bit-bottom-first-all-ac",
            "avc-field-transform-8bit-top-first-t8-qp40-scale24",
            "avc-field-transform-10bit-bottom-first-t4-qp18-scale24",
            "avc-field-bypass-8bit-top-first-t8-enabled-scale24",
            "avc-field-bypass-10bit-bottom-first-t4-enabled-scale24",
            "avc-field-filter-8bit-top-first-motion-filter0",
            "avc-field-filter-10bit-bottom-first-all-ac-filter2",
            "avc-field-multislice-8bit-top-first-motion-filter0-aso",
            "avc-field-multislice-10bit-bottom-first-all-ac-filter2-aso",
            "avc-field-rows-8bit-top-first-all-ac-one-filter0",
            "avc-field-rows-10bit-bottom-first-motion-rows-filter2-aso",
            "avc-field-refs-8bit-top-first-normal-coded-filter0-aso",
            "avc-field-refs-10bit-bottom-first-swap-skip-filter2-aso",
            "avc-field-opposite-8bit-top-first-normal-coded-filter0-aso",
            "avc-field-opposite-10bit-bottom-first-swap-skip-filter2-aso",
            "avc-field-multiref-8bit-top-first-type3-sub3-r0-filter0-aso",
            "avc-field-multiref-10bit-bottom-first-type1-sub0-r1-filter2-aso",
            "avc-field-b-cabac-mixed-8bit-top-first-spatial-l0-pos0-coded-v1-init0-filter0-infer1",
            "avc-field-b-cabac-mixed-10bit-bottom-first-temporal-i16-negative-pos1-skip-v1-init2-filter2-infer0",
            "avc-field-b-cabac-mixed-8bit-top-first-spatial-i4-zero-pos1-coded-v1-init1-filter1-infer0",
            "avc-field-b-cabac-mixed-10bit-bottom-first-spatial-bi-pos1-skip-v1-init2-filter2-infer1",
            "avc-field-b-cabac-residual-8bit-top-first-temporal-all-sign1-init0-filter0-infer1-aso",
            "avc-field-b-cabac-residual-10bit-bottom-first-spatial-chroma-sign1-init2-filter2-infer0-aso",
            "avc-field-b-cabac-sub-references-8bit-top-first-temporal-type12-pos0-init0-filter0-infer0",
            "avc-field-b-cabac-sub-references-10bit-bottom-first-spatial-type9-pos3-init2-filter2-infer1",
            "avc-field-b-cabac-sub-joined-8bit-top-first-temporal-type12-pos0-init0-filter0-infer0",
            "avc-field-b-cabac-sub-joined-10bit-bottom-first-spatial-type9-pos3-init2-filter2-infer1",
            "avc-field-b-cabac-sub-8bit-top-first-temporal-type12-pos0-init0-filter0-infer0",
            "avc-field-b-cabac-sub-10bit-bottom-first-spatial-type9-pos3-init2-filter2-infer1",
            "avc-field-b-gap-longterm-poc1-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-poc1-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-mixed-poc1-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-mixed-poc1-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-poc2-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-poc2-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-mixed-poc2-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-mixed-poc2-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-wrap-poc1-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-wrap-poc1-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-wrap-mixed-poc1-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-wrap-mixed-poc1-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-wrap-poc2-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-wrap-poc2-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-wrap-mixed-poc2-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-wrap-mixed-poc2-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-wrap-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-wrap-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-wrap-mixed-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-wrap-mixed-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-reset-gap-poc1-8bit-top-first-temporal-coded-cabac-init0-filter0-initiallong",
            "avc-field-b-reset-gap-poc1-10bit-bottom-first-spatial-skip-cavlc-filter2-initialshort",
            "avc-field-b-reset-gap-poc2-8bit-top-first-temporal-coded-cabac-init0-filter0-initiallong",
            "avc-field-b-reset-gap-poc2-10bit-bottom-first-spatial-skip-cavlc-filter2-initialshort",
            "avc-frame-to-field-long-8bit-top-first-coded-filter0",
            "avc-frame-to-field-long-10bit-bottom-first-skip-filter2",
            "avc-paff-intra-joined-8bit-i4-bias-filter1",
            "avc-paff-intra-joined-10bit-i16-negative-filter2",
            "avc-paff-intra-8bit-i4-ac-filter0",
            "avc-paff-intra-10bit-i16-negative-filter2-aso",
            "avc-paff-frame-to-field-8bit-top-first-coded-filter0",
            "avc-paff-frame-to-field-10bit-bottom-first-skip-filter2",
            "avc-paff-frame-to-field-long-8bit-top-first-coded-filter0",
            "avc-paff-frame-to-field-long-10bit-bottom-first-skip-filter2",
            "avc-frame-to-field-8bit-top-first-coded-filter0",
            "avc-frame-to-field-10bit-bottom-first-skip-filter2",
            "avc-field-b-reset-gap-8bit-top-first-temporal-coded-cabac-init0-filter0-initiallong",
            "avc-field-b-reset-gap-10bit-bottom-first-spatial-skip-cavlc-filter2-initialshort",
            "avc-field-b-gap-longterm-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-gap-longterm-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-gap-longterm-mixed-8bit-top-first-temporal-source-long-coded-cavlc-filter0",
            "avc-field-b-gap-longterm-mixed-10bit-bottom-first-spatial-colocated-long-skip-cabac-init2-filter2",
            "avc-field-b-longterm-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-longterm-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-longterm-mixed-8bit-top-first-temporal-source-long-coded-cabac-init0-filter0",
            "avc-field-b-longterm-mixed-10bit-bottom-first-spatial-colocated-long-skip-cavlc-filter2",
            "avc-field-b-bypass8-8bit-top-first-temporal-ac-sign1-init0-filter0-scale24-enabled-aso",
            "avc-field-b-bypass8-10bit-bottom-first-spatial-ac-sign0-init2-filter2-scale16-enabled",
            "avc-field-gap-wrap-long-8bit-top-first-skip-filter1",
            "avc-field-gap-wrap-long-10bit-bottom-first-coded-filter2-aso",
            "avc-field-gap-wrap-8bit-top-first-skip-filter1",
            "avc-field-gap-wrap-10bit-bottom-first-coded-filter2-aso",
            "avc-field-gap-cabac-8bit-top-first-skip-filter1-init0",
            "avc-field-gap-cabac-10bit-bottom-first-coded-filter2-init2-aso",
            "avc-field-gap-wrap-cabac-8bit-top-first-skip-filter1-init1",
            "avc-field-gap-wrap-long-cabac-10bit-bottom-first-coded-filter2-init2-aso",
            "avc-field-gap-poc1-8bit-top-first-skip-filter1",
            "avc-field-gap-cabac-poc2-10bit-bottom-first-coded-filter2-init2-aso",
            "avc-field-gap-wrap-long-cabac-poc1-10bit-bottom-first-coded-filter2-init2-aso",
            "avc-field-gap-wrap-poc2-8bit-top-first-coded-filter0-aso",
            "avc-field-gap-8bit-top-first-skip-filter1",
            "avc-field-gap-10bit-bottom-first-coded-filter2-aso",
            "avc-field-b-transform8-8bit-top-first-temporal-ac-sign1-init0-filter0-scale24-aso",
            "avc-field-b-transform8-10bit-bottom-first-spatial-ac-sign0-init2-filter2-scale16-aso",
            "avc-field-b-residual-8bit-top-first-spatial-all-ac-pos0-sign1-filter0-infer1",
            "avc-field-b-residual-10bit-bottom-first-temporal-chroma-dc-pos1-sign1-filter2-infer0",
            "avc-field-b-mixed-8bit-top-first-spatial-adjacent-type3-pos0-v1-filter0-infer1",
            "avc-field-b-mixed-10bit-bottom-first-temporal-pcm-type0-pos0-v1-filter2-infer0",
            "avc-field-b-mixed-8bit-top-first-spatial-sub-type12-pos3-v1-filter0-infer1",
            "avc-field-b-mixed-10bit-bottom-first-temporal-sub-type9-pos1-v1-filter2-infer0",
            "avc-field-b-direct-8bit-top-first-temporal-coded-cavlc-filter0-infer0",
            "avc-field-b-direct-10bit-bottom-first-spatial-skip-cabac-init2-filter2-infer1-aso",
            "avc-field-b-direct-8bit-top-first-temporal-skip-cabac-init0-filter1-infer1",
            "avc-field-b-direct-10bit-bottom-first-spatial-coded-cavlc-filter1-infer0-aso",
            "avc-field-b-implicit-8bit-top-first-bi-cabac-init0-filter0-aso",
            "avc-field-b-implicit-10bit-bottom-first-l1-cavlc-filter2-aso",
            "avc-field-b-future-8bit-top-first-bi-cabac-init0-filter0-aso",
            "avc-field-b-future-10bit-bottom-first-l1-cavlc-filter2-aso",
            "avc-field-b-explicit-8bit-top-first-bi-cabac-init0-filter0-aso",
            "avc-field-b-explicit-10bit-bottom-first-l1-cavlc-filter2-aso",
            "avc-field-cabac-p-weight-8bit-top-first-skip-weighted-init0-filter0-aso",
            "avc-field-cabac-p-weight-10bit-bottom-first-residual-weighted-init2-filter2-aso",
            "avc-field-cabac-p-multiref-8bit-top-first-4x4-r0-list0-init0-filter0-aso",
            "avc-field-cabac-p-multiref-10bit-bottom-first-16x8-r1-list1-init2-filter2-aso",
            "avc-field-cabac-p-mixed-constrained-8bit-top-first-intra-last-i4-zero-skip-init0-filter0",
            "avc-field-cabac-p-mixed-constrained-10bit-bottom-first-intra-last-i16-negative-coded-init2-filter2",
            "avc-field-cabac-p-mixed-8bit-top-first-intra-first-i4-zero-skip-init0-filter0",
            "avc-field-cabac-p-mixed-10bit-bottom-first-intra-last-i16-negative-coded-init2-filter2",
            "avc-field-cabac-p-partition-8bit-top-first-4x4-init0-filter0-aso",
            "avc-field-cabac-p-partition-10bit-bottom-first-16x8-init2-filter2-aso",
            "avc-field-cabac-p-transform8-8bit-top-first-ac-init0-filter0-scale24-aso",
            "avc-field-cabac-p-transform8-10bit-bottom-first-ac-init2-filter2-scale16-aso",
            "avc-field-cabac-p-chroma-8bit-top-first-ac-init0-filter0-aso",
            "avc-field-cabac-p-chroma-10bit-bottom-first-ac-init2-filter2-aso",
            "avc-field-cabac-p-residual-8bit-top-first-ac-init0-filter0-aso",
            "avc-field-cabac-p-residual-10bit-bottom-first-ac-init2-filter2-aso",
            "avc-field-cabac-p-8bit-top-first-skip-init0-filter0-aso",
            "avc-field-cabac-p-10bit-bottom-first-coded-init2-filter2-aso",
            "avc-field-cabac-8bit-top-first-positive-filter0-aso",
            "avc-field-cabac-10bit-bottom-first-negative-filter2-aso",
            "avc-field-fmo-8bit-type0-dir0-top-first-dc1-filter0-aso",
            "avc-field-fmo-10bit-type3-dir1-bottom-first-dc1-filter2-aso",
            "avc-field-intra-bypass-8bit-top-first-i4-ac-filter1-scale8-enabled-aso",
            "avc-field-intra-bypass-10bit-bottom-first-i8-ac-filter1-scale16-enabled-aso",
            "avc-field-intra-bypass-10bit-bottom-first-i16-negative-filter1-scale8-control",
            "avc-field-intra8-8bit-top-first-i8-ac-filter0-scale8-aso",
            "avc-field-intra8-10bit-bottom-first-i8-ac-filter2-scale16-aso",
            "avc-field-intra-chroma-8bit-top-first-i4-ac-filter0-aso",
            "avc-field-intra-chroma-10bit-bottom-first-i16-negative-filter2-aso",
            "avc-field-intra-8bit-top-first-i4-ac-filter0-aso",
            "avc-field-intra-10bit-bottom-first-i16-negative-filter2-aso",
            "avc-field-intra-residual-8bit-top-first-intra-first-skip-i4-ac-filter0",
            "avc-field-intra-residual-10bit-bottom-first-intra-last-coded-i16-negative-filter2",
            "avc-field-mixed-intra4-8bit-top-first-intra-first-skip-filter0",
            "avc-field-mixed-intra4-10bit-bottom-first-intra-last-coded-filter2",
            "avc-field-mixed-pcm-8bit-top-first-pcm-first-skip-filter0",
            "avc-field-mixed-pcm-10bit-bottom-first-pcm-last-coded-filter2",
            "avc-field-weight-8bit-top-first-whole-skip-weighted-filter0",
            "avc-field-weight-10bit-bottom-first-residual-weighted-filter2-aso",
        ] {
            let oracle = std::fs::read(root.join(format!("{name}.yuv"))).unwrap();
            let paff_intra = name.starts_with("avc-paff-intra-");
            let frame_to_fields = name.starts_with("avc-frame-to-field-")
                || name.starts_with("avc-paff-frame-to-field-");
            let reset_gap = name.starts_with("avc-field-b-reset-gap-");
            let frame_gap_wrap = name.starts_with("avc-field-gap-wrap-");
            let frame_gap = name.starts_with("avc-field-gap-");
            let reordered_four = name.starts_with("avc-field-b-direct-")
                || name.starts_with("avc-field-b-mixed-")
                || name.starts_with("avc-field-b-residual-")
                || name.starts_with("avc-field-b-cabac-mixed-")
                || name.starts_with("avc-field-b-cabac-residual-")
                || name.starts_with("avc-field-b-transform8-")
                || name.starts_with("avc-field-b-bypass8-")
                || name.starts_with("avc-field-b-longterm-")
                || name.starts_with("avc-field-b-gap-")
                || name.starts_with("avc-field-b-cabac-sub-");
            let count = if paff_intra {
                1
            } else if reset_gap {
                5
            } else if reordered_four {
                4
            } else if (name.starts_with("avc-field-cabac-")
                || name.starts_with("avc-field-fmo-")
                || name.starts_with("avc-field-intra-")
                || name.starts_with("avc-field-intra8-"))
                && !name.starts_with("avc-field-intra-residual-")
                && !name.starts_with("avc-field-cabac-p-")
            {
                1
            } else if frame_gap
                || name.starts_with("avc-field-b-")
                || name.contains("refs")
                || name.contains("opposite")
                || name.contains("multiref")
                || name.contains("weight")
            {
                3
            } else if name.contains("skip")
                || name.contains("motion")
                || name.contains("partition")
                || name.contains("residual")
                || name.contains("transform")
                || name.contains("bypass")
                || name.contains("filter")
                || name.contains("multislice")
                || name.contains("rows")
            {
                2
            } else {
                1
            };
            let frame_bytes = oracle.len() / count;
            let mut calls = 0usize;
            let mut visitor = |frame: &FrameMetadata, pixels: &[u8], start, duration| {
                assert_eq!(
                    (frame.sample, frame.pts, frame.duration),
                    (
                        if frame_to_fields {
                            [0, 1][calls]
                        } else if reset_gap {
                            [0, 2, 4, 8, 6][calls]
                        } else if reordered_four {
                            [0, 2, 6, 4][calls]
                        } else if name.starts_with("avc-field-b-future-")
                            || name.starts_with("avc-field-b-implicit-")
                        {
                            [0, 4, 2][calls]
                        } else {
                            calls * 2
                        },
                        if reordered_four {
                            [0, 4, 6, 8][calls]
                        } else if frame_gap {
                            if frame_gap_wrap {
                                [0, 2, 4][calls]
                            } else {
                                [0, 4, 6][calls]
                            }
                        } else {
                            calls as i64 * 2
                        },
                        if paff_intra {
                            1
                        } else if (reordered_four || (frame_gap && !frame_gap_wrap)) && calls == 0 {
                            4
                        } else {
                            2
                        }
                    )
                );
                assert_eq!(
                    (start, duration),
                    (
                        if reordered_four {
                            [0, 4, 6, 8][calls] * 20_000_000
                        } else if frame_gap {
                            (if frame_gap_wrap {
                                [0, 2, 4][calls]
                            } else {
                                [0, 4, 6][calls]
                            }) * 20_000_000
                        } else {
                            calls as u64 * 40_000_000
                        },
                        if paff_intra {
                            20_000_000
                        } else if (reordered_four || (frame_gap && !frame_gap_wrap)) && calls == 0 {
                            80_000_000
                        } else {
                            40_000_000
                        }
                    )
                );
                assert_eq!(
                    pixels,
                    &oracle[calls * frame_bytes..(calls + 1) * frame_bytes]
                );
                calls += 1;
                Ok(())
            };
            let stats = decode_presented(
                &root.join(format!("{name}.mp4")),
                &DecodeTransform::default(),
                Some(&mut visitor),
                None,
            )
            .unwrap()
            .unwrap();
            assert_eq!(stats.video_frames, count as u64);
            assert_eq!(calls, count);
        }
        assert!(
            try_decode(
                &root.join("avc-field-pcm-unpaired.mp4"),
                &DecodeTransform::default()
            )
            .unwrap_err()
            .contains("unpaired AVC field at end of MP4")
        );
    }
    #[test]
    fn duplicate_pts_replace_metadata_and_keep_terminal_nominal_span() {
        let mut frames = vec![frame(2, 1, 0), frame(0, 0, 1), frame(2, 2, 2)];
        normalize_presentations(&mut frames).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].pts, frames[0].duration), (0, 2));
        assert_eq!(
            (frames[1].pts, frames[1].duration, frames[1].size),
            (2, 3, [3, 1])
        );
        assert!(normalize_presentations(&mut vec![frame(0, 0, 0)]).is_err());
        assert!(normalize_presentations(&mut vec![frame(i64::MAX, 1, 0)]).is_err());
    }
}

fn check_options(options: Option<&fvid_control::CopyOptions>, sample: usize) -> Result<()> {
    if options
        .and_then(|o| o.cancel.as_ref())
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if options
        .and_then(|o| o.max_packets)
        .is_some_and(|n| sample as u64 >= n)
    {
        return Err("MP4 input packet count exceeds limit".into());
    }
    Ok(())
}
fn emit(
    frame: &FrameMetadata,
    start: i64,
    end: i64,
    scale: u32,
    spool: &mut Option<crate::owned_reverse::Reverse>,
    pixels: &mut Vec<u8>,
    visit: &mut Option<&mut Visitor<'_>>,
    stats: &mut DecodeStats,
) -> Result<()> {
    frame.account(stats)?;
    if let (Some(spool), Some(callback)) = (spool.as_mut(), visit.as_deref_mut()) {
        if scale == 0 || start < 0 || end <= start {
            return Err("invalid MP4 presentation interval".into());
        }
        let ns = |ticks: i64| {
            u64::try_from(i128::from(ticks) * 1_000_000_000 / i128::from(scale))
                .map_err(|_| "MP4 timestamp exceeds nanosecond range".to_string())
        };
        let start = ns(start)?;
        let end = ns(end)?;
        if end <= start {
            return Err("MP4 interval is below nanosecond precision".into());
        }
        spool.read_at(frame.slot, pixels)?;
        callback(frame, pixels, start, end - start)?;
    }
    Ok(())
}
fn pack_avc(p: &fvid_codecs::codec::avc_picture::IntraPicture) -> Result<Vec<u8>> {
    pack_cropped(
        [p.coded_width, p.coded_height],
        p.crop,
        [p.bit_depth; 3],
        [true, true],
        false,
        [&p.y, &p.cb, &p.cr],
    )
}
fn pack_hevc(
    p: &fvid_codecs::codec::hevc_picture::Picture,
    sub: [bool; 2],
    mono: bool,
) -> Result<Vec<u8>> {
    pack_cropped(
        p.dimensions.map(|v| v as usize),
        p.crop.map(|v| v as usize),
        [p.depth[0], p.depth[1], p.depth[1]],
        sub,
        mono,
        p.planes.each_ref().map(|plane| plane.samples()),
    )
}
fn pack_cropped(
    coded: [usize; 2],
    crop: [usize; 4],
    depths: [u8; 3],
    sub: [bool; 2],
    mono: bool,
    planes: [&[u16]; 3],
) -> Result<Vec<u8>> {
    if depths.iter().any(|&d| !(8..=16).contains(&d)) {
        return Err("invalid component depth".into());
    }
    let depth = if mono {
        depths[0]
    } else {
        *depths.iter().max().unwrap()
    };
    let width = coded[0]
        .checked_sub(crop[0])
        .and_then(|n| n.checked_sub(crop[1]))
        .ok_or("video crop exceeds width")?;
    let height = coded[1]
        .checked_sub(crop[2])
        .and_then(|n| n.checked_sub(crop[3]))
        .ok_or("video crop exceeds height")?;
    let sub = if mono {
        [1, 1]
    } else {
        sub.map(|v| if v { 2 } else { 1 })
    };
    let chroma = width
        .div_ceil(sub[0])
        .checked_mul(height.div_ceil(sub[1]))
        .ok_or("chroma size overflow")?;
    let count = width
        .checked_mul(height)
        .and_then(|n| chroma.checked_mul(2).and_then(|c| n.checked_add(c)))
        .ok_or("video size overflow")?;
    let bytes = if depth == 8 { 1 } else { 2 };
    let mut output =
        crate::owned_frame::buffer(count.checked_mul(bytes).ok_or("video size overflow")?)?;
    let mut cursor = 0;
    for (component, plane) in planes.iter().enumerate() {
        let (sx, sy) = if component == 0 {
            (1, 1)
        } else {
            (sub[0], sub[1])
        };
        let stride = coded[0].div_ceil(sx);
        for row in 0..height.div_ceil(sy) {
            for column in 0..width.div_ceil(sx) {
                let sample = if mono && component > 0 {
                    1u16 << (depth - 1)
                } else {
                    let index = (crop[2] / sy + row)
                        .checked_mul(stride)
                        .and_then(|v| v.checked_add(crop[0] / sx + column))
                        .ok_or("video sample offset overflow")?;
                    let sample = *plane.get(index).ok_or("decoded crop exceeds plane")?;
                    if u32::from(sample) >= (1u32 << depths[component]) {
                        return Err("component sample exceeds depth".into());
                    }
                    sample << (depth - depths[component])
                };
                if depth == 8 {
                    output[cursor] = u8::try_from(sample).map_err(|_| "8-bit sample overflow")?;
                    cursor += 1;
                } else {
                    output[cursor..cursor + 2].copy_from_slice(&sample.to_le_bytes());
                    cursor += 2;
                }
            }
        }
    }
    Ok(output)
}
