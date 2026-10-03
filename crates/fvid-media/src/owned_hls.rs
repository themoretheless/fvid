//! HLS manifest parsing and finite fMP4 assembly, independent of network and codecs.
//! RFC 8216 URIs stay unresolved until the input transport supplies a base URL.
use std::io::{Read, Seek, SeekFrom, Write};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resource<'a> {
    pub uri: &'a str,
    pub range: Option<ByteRange>,
}
#[derive(Clone, Copy, Debug)]
pub struct Segment<'a> {
    pub resource: Resource<'a>,
    pub duration: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Variant<'a> {
    pub uri: &'a str,
    pub bandwidth: u64,
    pub external_renditions: bool,
}
#[derive(Debug)]
pub struct MediaPlaylist<'a> {
    pub initialization: Option<Resource<'a>>,
    pub segments: Vec<Segment<'a>>,
    pub end_list: bool,
    pub media_sequence: u64,
}
#[derive(Debug)]
pub enum Playlist<'a> {
    Master(Vec<Variant<'a>>),
    Media(MediaPlaylist<'a>),
}

fn attributes(text: &str) -> Result<Vec<(&str, &str)>> {
    let mut result = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let (name, rest) = remaining
            .split_once('=')
            .ok_or("invalid HLS attribute list")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid HLS attribute name".into());
        }
        if result.iter().any(|(key, _)| *key == name) {
            return Err("duplicate HLS attribute".into());
        }
        let (value, tail) = if let Some(rest) = rest.strip_prefix('"') {
            let end = rest.find('"').ok_or("unterminated HLS quoted attribute")?;
            (&rest[..end], &rest[end + 1..])
        } else {
            rest.split_once(',')
                .map_or((rest, ""), |(value, tail)| (value, tail))
        };
        result.try_reserve(1).map_err(|e| e.to_string())?;
        result.push((name, value));
        if rest.starts_with('"') {
            remaining = if tail.is_empty() {
                ""
            } else {
                tail.strip_prefix(',')
                    .ok_or("invalid HLS attribute separator")?
            };
        } else {
            remaining = tail;
        }
        if remaining.is_empty() && text.ends_with(',') {
            return Err("empty HLS attribute".into());
        }
    }
    Ok(result)
}
fn attribute<'a>(attrs: &[(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find_map(|(key, value)| (*key == name).then_some(*value))
}
fn positive(text: &str) -> Result<u64> {
    let value: u64 = text.parse().map_err(|_| "invalid HLS positive integer")?;
    if value == 0 {
        return Err("HLS positive integer must not be zero".into());
    }
    Ok(value)
}
fn range(text: &str) -> Result<(u64, Option<u64>)> {
    let (length, offset) = text
        .split_once('@')
        .map_or((text, None), |(length, offset)| (length, Some(offset)));
    let length = positive(length)?;
    let offset = offset
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| "invalid HLS range offset")?;
    if offset.is_some_and(|offset| offset.checked_add(length).is_none()) {
        return Err("HLS byte range overflow".into());
    }
    Ok((length, offset))
}
fn resource<'a>(
    uri: &'a str,
    bytes: Option<(u64, Option<u64>)>,
    previous: Option<Resource<'_>>,
) -> Result<Resource<'a>> {
    if uri.is_empty() || uri.contains(['\0', '\r', '\n', '{', '}']) {
        return Err("invalid or unsupported HLS resource URI".into());
    }
    let range = if let Some((length, offset)) = bytes {
        let offset=offset.or_else(||previous.filter(|previous|previous.uri==uri).and_then(|p|p.range).and_then(|r|r.offset.checked_add(r.length))).ok_or("implicit HLS byte range requires the preceding ranged segment of the same resource")?;
        offset
            .checked_add(length)
            .ok_or("HLS byte range overflow")?;
        Some(ByteRange { offset, length })
    } else {
        None
    };
    Ok(Resource { uri, range })
}
/// Parse master or media manifests. Unsupported media-changing tags are refused,
/// rather than silently dropping encryption, discontinuities or missing segments.
pub fn parse(text: &str) -> Result<Playlist<'_>> {
    let mut lines = text.lines();
    if lines.next() != Some("#EXTM3U") {
        return Err("HLS playlist must begin with EXTM3U".into());
    }
    let mut variants = Vec::new();
    let mut segments = Vec::<Segment<'_>>::new();
    let mut initialization = None;
    let mut pending_duration = None;
    let mut pending_range = None;
    let mut pending_variant = None;
    let mut end_list = false;
    let mut media_sequence = 0;
    let mut target = None;
    let mut version = 1;
    let mut has_external = false;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.contains('\0') {
            return Err("NUL in HLS playlist".into());
        }
        if let Some(value) = line.strip_prefix("#EXT-X-VERSION:") {
            version = positive(value)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            target = Some(positive(value)?);
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if !segments.is_empty() {
                return Err("HLS media sequence must precede segments".into());
            }
            media_sequence = value.parse().map_err(|_| "invalid HLS media sequence")?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            if pending_variant.is_some() || pending_duration.is_some() {
                return Err("missing HLS URI before stream declaration".into());
            }
            let attrs = attributes(value)?;
            let bandwidth =
                positive(attribute(&attrs, "BANDWIDTH").ok_or("HLS variant lacks BANDWIDTH")?)?;
            pending_variant = Some((
                bandwidth,
                attribute(&attrs, "AUDIO").is_some()
                    || attribute(&attrs, "VIDEO").is_some()
                    || attribute(&attrs, "SUBTITLES").is_some(),
            ));
        } else if line.starts_with("#EXT-X-MEDIA:") {
            has_external = true;
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            if pending_duration.is_some() || pending_variant.is_some() {
                return Err("missing HLS URI before EXTINF".into());
            }
            let duration: f64 = value
                .split(',')
                .next()
                .unwrap()
                .parse()
                .map_err(|_| "invalid HLS segment duration")?;
            if !duration.is_finite() || duration <= 0.0 {
                return Err("invalid HLS segment duration".into());
            }
            pending_duration = Some(duration);
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            if pending_range.is_some() {
                return Err("duplicate HLS segment byte range".into());
            }
            pending_range = Some(range(value)?);
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            let attrs = attributes(value)?;
            let uri = attribute(&attrs, "URI").ok_or("HLS map lacks URI")?;
            let bytes = attribute(&attrs, "BYTERANGE").map(range).transpose()?;
            let map = resource(uri, bytes, None)?;
            if initialization.is_some_and(|previous| previous != map) {
                return Err("changing HLS initialization is not yet supported".into());
            }
            if !segments.is_empty() && initialization.is_none() {
                return Err("HLS initialization must precede its segments".into());
            }
            initialization = Some(map);
        } else if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            let attrs = attributes(value)?;
            if attribute(&attrs, "METHOD") != Some("NONE") {
                return Err("encrypted HLS is not yet supported".into());
            }
        } else if line == "#EXT-X-ENDLIST" {
            end_list = true;
        } else if matches!(
            line,
            "#EXT-X-DISCONTINUITY" | "#EXT-X-GAP" | "#EXT-X-I-FRAMES-ONLY"
        ) {
            return Err(
                "HLS discontinuity, gap or I-frame-only playback is not yet supported".into(),
            );
        } else if line.starts_with("#EXT-X-PART:")
            || line.starts_with("#EXT-X-SKIP:")
            || line.starts_with("#EXT-X-DEFINE:")
        {
            return Err("HLS partial segments, delta updates and variable substitution are not yet supported".into());
        } else if !line.starts_with('#') {
            if end_list {
                return Err("HLS segment follows ENDLIST".into());
            }
            if let Some((bandwidth, external_renditions)) = pending_variant.take() {
                resource(line, None, None)?;
                variants.try_reserve(1).map_err(|e| e.to_string())?;
                variants.push(Variant {
                    uri: line,
                    bandwidth,
                    external_renditions,
                });
            } else {
                let duration = pending_duration.take().ok_or("HLS segment lacks EXTINF")?;
                let resource = resource(
                    line,
                    pending_range.take(),
                    segments.last().map(|s| s.resource),
                )?;
                segments.try_reserve(1).map_err(|e| e.to_string())?;
                segments.push(Segment { resource, duration });
            }
        }
    }
    if pending_variant.is_some() || pending_duration.is_some() || pending_range.is_some() {
        return Err("HLS playlist ends before a resource URI".into());
    }
    if !variants.is_empty() {
        if !segments.is_empty() || initialization.is_some() || target.is_some() || end_list {
            return Err("mixed HLS master and media playlist".into());
        }
        if has_external {
            for variant in &mut variants {
                variant.external_renditions = true;
            }
        }
        return Ok(Playlist::Master(variants));
    }
    if segments.is_empty() {
        return Err("HLS playlist contains no segments".into());
    }
    let target = target.ok_or("HLS media playlist lacks TARGETDURATION")?;
    if segments
        .iter()
        .any(|segment| segment.duration.round() > target as f64)
    {
        return Err("HLS segment exceeds TARGETDURATION".into());
    }
    if initialization.is_some() && version < 6 {
        return Err("fMP4 HLS map requires playlist version 6 or later".into());
    }
    Ok(Playlist::Media(MediaPlaylist {
        initialization,
        segments,
        end_list,
        media_sequence,
    }))
}

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
/// Append initialization and fragment byte ranges in playlist order. Network and
/// file access belong to the fetch callback; the parser/assembler needs neither.
pub fn assemble_fmp4(
    playlist: &MediaPlaylist<'_>,
    output: &mut impl Write,
    mut fetch: impl FnMut(&Resource<'_>) -> Result<Box<dyn ReadSeek>>,
    byte_limit: Option<u64>,
    cancel: Option<&fvid_control::CancelFlag>,
) -> Result<u64> {
    if !playlist.end_list {
        return Err("live HLS reload is not yet supported".into());
    }
    let initialization = playlist
        .initialization
        .as_ref()
        .ok_or("HLS MPEG-TS/packed-audio assembly is not yet supported; fMP4 requires EXT-X-MAP")?;
    let mut total = 0u64;
    let mut scratch = [0u8; 64 * 1024];
    for resource in
        std::iter::once(initialization).chain(playlist.segments.iter().map(|s| &s.resource))
    {
        if cancel.is_some_and(fvid_control::CancelFlag::is_cancelled) {
            return Err("HLS assembly cancelled".into());
        }
        let mut input = fetch(resource)?;
        let size = input.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        let range = resource.range.unwrap_or(ByteRange {
            offset: 0,
            length: size,
        });
        if range.length == 0
            || range
                .offset
                .checked_add(range.length)
                .is_none_or(|end| end > size)
        {
            return Err("HLS byte range is outside its resource".into());
        }
        if total
            .checked_add(range.length)
            .is_none_or(|n| byte_limit.is_some_and(|limit| n > limit))
        {
            return Err("HLS assembly exceeds byte limit or size range".into());
        }
        input
            .seek(SeekFrom::Start(range.offset))
            .map_err(|e| e.to_string())?;
        let mut remaining = range.length;
        while remaining != 0 {
            if cancel.is_some_and(fvid_control::CancelFlag::is_cancelled) {
                return Err("HLS assembly cancelled".into());
            }
            let count = (remaining.min(scratch.len() as u64)) as usize;
            input
                .read_exact(&mut scratch[..count])
                .map_err(|e| format!("truncated HLS resource: {e}"))?;
            output
                .write_all(&scratch[..count])
                .map_err(|e| e.to_string())?;
            remaining -= count as u64;
            total += count as u64;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_mp4::{Limits, Mp4Reader};
    use std::{fs::File, io::Cursor, path::PathBuf};
    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/hls")
    }
    #[test]
    fn finite_fmp4_resources_and_ranges_preserve_every_packet_and_timestamp() {
        for manifest in ["media.m3u8", "byterange.m3u8"] {
            let text = std::fs::read_to_string(fixtures().join(manifest)).unwrap();
            let Playlist::Media(playlist) = parse(&text).unwrap() else {
                panic!("media")
            };
            let mut output = Vec::new();
            let bytes = assemble_fmp4(
                &playlist,
                &mut output,
                |r| Ok(Box::new(File::open(fixtures().join(r.uri)).unwrap())),
                None,
                None,
            )
            .unwrap();
            assert_eq!(bytes as usize, output.len());
            assert_eq!(
                output,
                std::fs::read(fixtures().join("objects.mp4")).unwrap()
            );
            let mut assembled = Mp4Reader::open(Cursor::new(output), Limits::default()).unwrap();
            let mut original = Mp4Reader::open(
                File::open(fixtures().join("../../fragmented/video.mp4")).unwrap(),
                Limits::default(),
            )
            .unwrap();
            assert_eq!(assembled.tracks().len(), 1);
            assert_eq!(assembled.tracks()[0].samples.len(), 25);
            for index in 0..25 {
                let a = assembled.tracks()[0].samples.get(index).unwrap();
                let b = original.tracks()[0].samples.get(index).unwrap();
                assert_eq!(
                    (a.dts, a.pts, a.duration, a.size, a.sync),
                    (b.dts, b.pts, b.duration, b.size, b.sync)
                );
                let (mut a, mut b) = (Vec::new(), Vec::new());
                assembled.read_packet(0, index, &mut a).unwrap();
                original.read_packet(0, index, &mut b).unwrap();
                assert_eq!(a, b);
            }
        }
    }
    #[test]
    fn quoted_codec_lists_and_external_renditions_are_retained() {
        let Playlist::Master(variants) = parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=200,CODECS=\"avc1,mp4a\",AUDIO=\"audio\"\nvideo.m3u8\n").unwrap() else { panic!("master") };
        assert_eq!(variants[0].bandwidth, 200);
        assert!(variants[0].external_renditions);
        assert_eq!(variants[0].uri, "video.m3u8");
    }
    #[test]
    fn unsupported_media_changes_and_invalid_ranges_refuse_specific_cause() {
        let good = std::fs::read_to_string(fixtures().join("media.m3u8")).unwrap();
        for (tag, reason) in [
            ("#EXT-X-KEY:METHOD=AES-128,URI=\"key\"", "encrypted"),
            ("#EXT-X-DISCONTINUITY", "discontinuity"),
            ("#EXT-X-GAP", "gap"),
        ] {
            let bad = good.replace("#EXTINF:0.2,", &format!("{tag}\n#EXTINF:0.2,"));
            assert!(parse(&bad).unwrap_err().contains(reason));
        }
        let bad = good.replace("segment-0.m4s", "#EXT-X-BYTERANGE:10\nsegment-0.m4s");
        assert!(parse(&bad)
            .unwrap_err()
            .contains("preceding ranged segment"));
        let live = good.replace("#EXT-X-ENDLIST", "");
        let Playlist::Media(playlist) = parse(&live).unwrap() else {
            panic!("media")
        };
        assert!(assemble_fmp4(
            &playlist,
            &mut Vec::new(),
            |_| panic!("must not fetch live"),
            None,
            None
        )
        .unwrap_err()
        .contains("live"));
    }
    #[test]
    fn assembly_checks_limit_cancellation_and_range_bounds() {
        let text = std::fs::read_to_string(fixtures().join("byterange.m3u8")).unwrap();
        let Playlist::Media(playlist) = parse(&text).unwrap() else {
            panic!("media")
        };
        let fetch = |r: &Resource<'_>| -> Result<Box<dyn ReadSeek>> {
            Ok(Box::new(File::open(fixtures().join(r.uri)).unwrap()))
        };
        assert!(
            assemble_fmp4(&playlist, &mut Vec::new(), fetch, Some(1), None)
                .unwrap_err()
                .contains("limit")
        );
        let cancel = fvid_control::CancelFlag::default();
        cancel.cancel();
        assert!(
            assemble_fmp4(&playlist, &mut Vec::new(), fetch, None, Some(&cancel))
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(assemble_fmp4(
            &playlist,
            &mut Vec::new(),
            |_| Ok(Box::new(Cursor::new(vec![0u8]))),
            None,
            None
        )
        .unwrap_err()
        .contains("outside"));
    }
}
