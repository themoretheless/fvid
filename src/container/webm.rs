//! Bounded seekable WebM/Matroska indexing, without an external demultiplexer.
use crate::{Result, invalid};
use std::io::{Read, Seek, SeekFrom};
const SEGMENT: u32 = 0x18538067;
const CLUSTER: u32 = 0x1f43b675;
#[derive(Clone, Debug)]
pub struct Track {
    pub number: u64,
    pub kind: u64,
    pub codec: String,
    pub width: u64,
    pub height: u64,
}
#[derive(Clone, Debug)]
pub struct Packet {
    pub track: u64,
    pub pts_ns: i64,
    pub keyframe: bool,
    pub offset: u64,
    pub size: usize,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub packets: usize,
    pub packet_bytes: usize,
    pub elements: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            packets: 1_000_000,
            packet_bytes: 32 << 20,
            elements: 4_000_000,
        }
    }
}
pub struct WebmReader<R> {
    reader: R,
    pub tracks: Vec<Track>,
    pub packets: Vec<Packet>,
    limits: Limits,
}
#[derive(Clone, Copy)]
struct Element {
    id: u32,
    data: u64,
    end: Option<u64>,
}
fn vint(r: &mut impl Read, id: bool) -> Result<(u64, bool)> {
    let mut first = [0];
    r.read_exact(&mut first)?;
    if first[0] == 0 {
        return Err(invalid("zero EBML variable integer"));
    }
    let len = first[0].leading_zeros() as usize + 1;
    if len > if id { 4 } else { 8 } {
        return Err(invalid("oversized EBML variable integer"));
    }
    let mut value = u64::from(if id {
        first[0]
    } else {
        first[0] & ((1u8 << (8 - len)) - 1)
    });
    for _ in 1..len {
        let mut b = [0];
        r.read_exact(&mut b)?;
        value = (value << 8) | u64::from(b[0]);
    }
    Ok((value, !id && value == (1u64 << (7 * len)) - 1))
}
fn element<R: Read + Seek>(
    r: &mut R,
    limit: u64,
    count: &mut usize,
    max: usize,
) -> Result<Element> {
    *count += 1;
    if *count > max {
        return Err(invalid("WebM element limit exceeded"));
    }
    let (id, _) = vint(r, true)?;
    let (size, unknown) = vint(r, false)?;
    let data = r.stream_position()?;
    if data > limit {
        return Err(invalid("truncated EBML element header"));
    }
    let end = if unknown {
        None
    } else {
        Some(
            data.checked_add(size)
                .filter(|&n| n <= limit)
                .ok_or_else(|| invalid("EBML element exceeds parent"))?,
        )
    };
    Ok(Element {
        id: id as u32,
        data,
        end,
    })
}
fn bytes<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Result<Vec<u8>> {
    let end = e
        .end
        .ok_or_else(|| invalid("unknown size for EBML value"))?;
    let len = usize::try_from(end - e.data)
        .ok()
        .filter(|&n| n <= max)
        .ok_or_else(|| invalid("EBML value exceeds limit"))?;
    r.seek(SeekFrom::Start(e.data))?;
    let mut out = vec![0; len];
    r.read_exact(&mut out)?;
    Ok(out)
}
fn uint<R: Read + Seek>(r: &mut R, e: Element) -> Result<u64> {
    Ok(bytes(r, e, 8)?
        .into_iter()
        .fold(0, |v, b| (v << 8) | u64::from(b)))
}
fn end(e: Element) -> Result<u64> {
    e.end
        .ok_or_else(|| invalid("unsupported unknown-sized EBML element"))
}
fn fields<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
) -> Result<Vec<Element>> {
    let limit = end(e)?;
    let mut at = e.data;
    let mut out = Vec::new();
    while at < limit {
        r.seek(SeekFrom::Start(at))?;
        let child = element(r, limit, count, max)?;
        at = end(child)?;
        out.push(child);
    }
    Ok(out)
}
impl<R: Read + Seek> WebmReader<R> {
    pub fn open(mut reader: R, limits: Limits) -> Result<Self> {
        let file_end = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut count = 0;
        let header = element(&mut reader, file_end, &mut count, limits.elements)?;
        if header.id != 0x1a45dfa3 {
            return Err(invalid("missing EBML header"));
        }
        let mut doctype = None;
        for e in fields(&mut reader, header, &mut count, limits.elements)? {
            if e.id == 0x4282 {
                doctype = Some(bytes(&mut reader, e, 16)?);
            }
        }
        if !matches!(doctype.as_deref(), Some(b"webm" | b"matroska")) {
            return Err(invalid("unsupported EBML document type"));
        }
        reader.seek(SeekFrom::Start(end(header)?))?;
        let segment = element(&mut reader, file_end, &mut count, limits.elements)?;
        if segment.id != SEGMENT {
            return Err(invalid("missing WebM Segment"));
        }
        let segment_end = segment.end.unwrap_or(file_end);
        let mut at = segment.data;
        let mut scale = 1_000_000u64;
        let mut tracks = Vec::new();
        let mut packets = Vec::new();
        while at < segment_end {
            reader.seek(SeekFrom::Start(at))?;
            let e = element(&mut reader, segment_end, &mut count, limits.elements)?;
            match e.id {
                0x1549a966 => {
                    for f in fields(&mut reader, e, &mut count, limits.elements)? {
                        if f.id == 0x2ad7b1 {
                            scale = uint(&mut reader, f)?;
                            if scale == 0 {
                                return Err(invalid("zero WebM timestamp scale"));
                            }
                        }
                    }
                }
                0x1654ae6b => {
                    for entry in fields(&mut reader, e, &mut count, limits.elements)? {
                        if entry.id != 0xae {
                            continue;
                        }
                        if tracks.len() >= 64 {
                            return Err(invalid("too many WebM tracks"));
                        }
                        let mut track = Track {
                            number: 0,
                            kind: 0,
                            codec: String::new(),
                            width: 0,
                            height: 0,
                        };
                        for f in fields(&mut reader, entry, &mut count, limits.elements)? {
                            match f.id {
                                0xd7 => track.number = uint(&mut reader, f)?,
                                0x83 => track.kind = uint(&mut reader, f)?,
                                0x86 => {
                                    track.codec = String::from_utf8(bytes(&mut reader, f, 128)?)
                                        .map_err(|_| invalid("invalid WebM codec ID"))?
                                }
                                0xe0 => {
                                    for v in fields(&mut reader, f, &mut count, limits.elements)? {
                                        match v.id {
                                            0xb0 => track.width = uint(&mut reader, v)?,
                                            0xba => track.height = uint(&mut reader, v)?,
                                            _ => {}
                                        }
                                    }
                                }
                                0x6d80 => {
                                    return Err(invalid(
                                        "encoded/encrypted WebM tracks not supported",
                                    ));
                                }
                                _ => {}
                            }
                        }
                        if track.number == 0
                            || tracks.iter().any(|t: &Track| t.number == track.number)
                            || track.codec.is_empty()
                        {
                            return Err(invalid("invalid WebM track"));
                        }
                        tracks.push(track);
                    }
                }
                CLUSTER => {
                    let cluster_end = e.end.unwrap_or(segment_end);
                    let mut pos = e.data;
                    let mut timestamp = None;
                    while pos < cluster_end {
                        reader.seek(SeekFrom::Start(pos))?;
                        let child = element(&mut reader, cluster_end, &mut count, limits.elements)?;
                        if e.end.is_none()
                            && matches!(
                                child.id,
                                CLUSTER
                                    | 0x1549a966
                                    | 0x1654ae6b
                                    | 0x1c53bb6b
                                    | 0x114d9b74
                                    | 0x1254c367
                                    | 0x1941a469
                                    | 0x1043a770
                            )
                        {
                            break;
                        }
                        match child.id {
                            0xe7 => timestamp = Some(uint(&mut reader, child)?),
                            0xa3 => read_block(
                                &mut reader,
                                child,
                                timestamp,
                                true,
                                true,
                                &mut packets,
                                limits,
                            )?,
                            0xa0 => {
                                let fs = fields(&mut reader, child, &mut count, limits.elements)?;
                                let key = !fs.iter().any(|f| f.id == 0xfb);
                                for block in fs {
                                    if block.id == 0xa1 {
                                        read_block(
                                            &mut reader,
                                            block,
                                            timestamp,
                                            false,
                                            key,
                                            &mut packets,
                                            limits,
                                        )?;
                                    }
                                }
                            }
                            _ => {}
                        }
                        pos = end(child)?;
                    }
                    at = pos;
                    continue;
                }
                _ => {}
            }
            at = end(e)?;
        }
        for p in &mut packets {
            p.pts_ns = i64::try_from(i128::from(p.pts_ns) * i128::from(scale))
                .map_err(|_| invalid("WebM timestamp overflow"))?;
            if !tracks.iter().any(|t| t.number == p.track) {
                return Err(invalid("WebM packet references missing track"));
            }
        }
        if tracks.is_empty() {
            return Err(invalid("WebM has no tracks"));
        }
        Ok(Self {
            reader,
            tracks,
            packets,
            limits,
        })
    }
    pub fn read_packet(&mut self, index: usize) -> Result<Vec<u8>> {
        let p = self
            .packets
            .get(index)
            .ok_or_else(|| invalid("WebM packet index out of bounds"))?;
        if p.size > self.limits.packet_bytes {
            return Err(invalid("WebM packet exceeds budget"));
        }
        self.reader.seek(SeekFrom::Start(p.offset))?;
        let mut data = vec![0; p.size];
        self.reader.read_exact(&mut data)?;
        Ok(data)
    }
}
fn read_block<R: Read + Seek>(
    r: &mut R,
    e: Element,
    timestamp: Option<u64>,
    simple: bool,
    key: bool,
    out: &mut Vec<Packet>,
    limits: Limits,
) -> Result<()> {
    if out.len() >= limits.packets {
        return Err(invalid("WebM packet count exceeds limit"));
    }
    r.seek(SeekFrom::Start(e.data))?;
    let (track, unknown) = vint(r, false)?;
    if track == 0 || unknown {
        return Err(invalid("invalid WebM block track"));
    }
    let mut h = [0; 3];
    r.read_exact(&mut h)?;
    let offset = r.stream_position()?;
    let limit = end(e)?;
    if offset >= limit {
        return Err(invalid("truncated WebM block"));
    }
    if h[2] & 6 != 0 {
        return Err(invalid("WebM laced blocks are not yet supported"));
    }
    let pts =
        i128::from(timestamp.ok_or_else(|| invalid("WebM block precedes Cluster timestamp"))?)
            + i128::from(i16::from_be_bytes([h[0], h[1]]));
    let size = usize::try_from(limit - offset)
        .ok()
        .filter(|&n| n <= limits.packet_bytes)
        .ok_or_else(|| invalid("WebM packet exceeds budget"))?;
    out.push(Packet {
        track,
        pts_ns: i64::try_from(pts).map_err(|_| invalid("WebM timestamp overflow"))?,
        keyframe: if simple { h[2] & 0x80 != 0 } else { key },
        offset,
        size,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn atom(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 127);
        [id, &[0x80 | payload.len() as u8], payload].concat()
    }
    fn fixture(lace: bool) -> Vec<u8> {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
        let track = atom(
            &[0xae],
            &[
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[1]),
                atom(&[0x86], b"V_VP9"),
                atom(
                    &[0xe0],
                    &[atom(&[0xb0], &[16]), atom(&[0xba], &[16])].concat(),
                ),
            ]
            .concat(),
        );
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        let cluster = |time: u8| {
            [
                vec![0x1f, 0x43, 0xb6, 0x75, 0xff],
                atom(&[0xe7], &[time]),
                atom(
                    &[0xa3],
                    &[0x81, 0xff, 0xff, if lace { 0x82 } else { 0x80 }, 0x82, 0x49],
                ),
            ]
            .concat()
        };
        [
            header,
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            tracks,
            cluster(2),
            cluster(4),
        ]
        .concat()
    }
    #[test]
    fn unknown_segment_and_cluster_sizes_signed_timestamps_and_packet_reads() {
        let mut r = WebmReader::open(Cursor::new(fixture(false)), Limits::default()).unwrap();
        assert_eq!(r.tracks[0].codec, "V_VP9");
        assert_eq!(r.tracks[0].width, 16);
        assert_eq!(r.packets.len(), 2);
        assert_eq!(r.packets[0].pts_ns, 1_000_000);
        assert_eq!(r.packets[1].pts_ns, 3_000_000);
        assert!(r.packets.iter().all(|p| p.keyframe));
        assert_eq!(r.read_packet(1).unwrap(), [0x82, 0x49]);
        assert!(r.read_packet(2).is_err());
    }
    #[test]
    fn malformed_vints_lacing_and_limits_fail() {
        assert!(vint(&mut Cursor::new([0]), false).is_err());
        assert!(vint(&mut Cursor::new([8, 0, 0, 0, 0]), true).is_err());
        assert!(WebmReader::open(Cursor::new(fixture(true)), Limits::default()).is_err());
        for limits in [
            Limits {
                packets: 1,
                ..Limits::default()
            },
            Limits {
                packet_bytes: 1,
                ..Limits::default()
            },
            Limits {
                elements: 2,
                ..Limits::default()
            },
        ] {
            assert!(WebmReader::open(Cursor::new(fixture(false)), limits).is_err());
        }
        let bytes = fixture(false);
        for len in 0..bytes.len() {
            let _ = WebmReader::open(Cursor::new(&bytes[..len]), Limits::default());
        }
    }
}
