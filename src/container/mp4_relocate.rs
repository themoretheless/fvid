//! Owned non-fragmented MP4 index relocation. Media and metadata stay encoded.
use crate::{Result, invalid};
use std::io::{Read, Seek, SeekFrom, Write};
#[derive(Clone)]
struct BoxSpan {
    start: u64,
    size: u64,
    header: u64,
    kind: [u8; 4],
}
fn header<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Result<BoxSpan> {
    if end - start < 8 {
        return Err(invalid("truncated MP4 box header"));
    }
    reader.seek(SeekFrom::Start(start))?;
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    let mut size = u64::from(u32::from_be_bytes(bytes[..4].try_into().unwrap()));
    let mut width = 8;
    if size == 1 {
        if end - start < 16 {
            return Err(invalid("truncated extended MP4 box"));
        }
        let mut extended = [0; 8];
        reader.read_exact(&mut extended)?;
        size = u64::from_be_bytes(extended);
        width = 16;
    } else if size == 0 {
        size = end - start;
    }
    if size < width || size > end - start {
        return Err(invalid("MP4 box exceeds input"));
    }
    Ok(BoxSpan {
        start,
        size,
        header: width,
        kind: bytes[4..].try_into().unwrap(),
    })
}
fn patch(data: &mut [u8], spans: &[BoxSpan], locations: &[u64], depth: usize) -> Result<()> {
    if depth > 16 {
        return Err(invalid("MP4 metadata nesting exceeds limit"));
    }
    let mut offset = 0;
    while offset < data.len() {
        let item = header(
            &mut std::io::Cursor::new(&*data),
            offset as u64,
            data.len() as u64,
        )?;
        let payload = &mut data[offset + item.header as usize..offset + item.size as usize];
        match &item.kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" => {
                patch(payload, spans, locations, depth + 1)?
            }
            b"meta" => patch(
                payload
                    .get_mut(4..)
                    .ok_or_else(|| invalid("truncated MP4 meta"))?,
                spans,
                locations,
                depth + 1,
            )?,
            b"mvex" | b"saio" | b"iloc" | b"rmra" => {
                return Err(invalid(
                    "MP4 relocation does not support these offset-bearing extensions",
                ));
            }
            b"stco" | b"co64" => {
                if payload.len() < 8 || payload[..4] != [0; 4] {
                    return Err(invalid("invalid MP4 chunk offset table"));
                }
                let count = u32::from_be_bytes(payload[4..8].try_into().unwrap()) as usize;
                let width = if item.kind == *b"stco" { 4 } else { 8 };
                if count.checked_mul(width).and_then(|n| n.checked_add(8)) != Some(payload.len()) {
                    return Err(invalid("invalid MP4 chunk offset count"));
                }
                for entry in payload[8..].chunks_exact_mut(width) {
                    let old = if width == 4 {
                        u64::from(u32::from_be_bytes(entry.try_into().unwrap()))
                    } else {
                        u64::from_be_bytes(entry.try_into().unwrap())
                    };
                    let index = spans
                        .iter()
                        .position(|span| {
                            span.kind == *b"mdat"
                                && old >= span.start + span.header
                                && old < span.start + span.size
                        })
                        .ok_or_else(|| invalid("MP4 chunk offset is outside media data"))?;
                    let new = locations[index]
                        .checked_add(old - spans[index].start)
                        .ok_or_else(|| invalid("MP4 chunk offset overflow"))?;
                    if width == 4 {
                        let new = u32::try_from(new).map_err(|_| {
                            invalid("MP4 relocation requires promoting stco to co64")
                        })?;
                        entry.copy_from_slice(&new.to_be_bytes());
                    } else {
                        entry.copy_from_slice(&new.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
        offset += item.size as usize;
    }
    Ok(())
}

/// Move moov before mdat and update stco/co64 without interpreting encoded media.
/// Preserves opaque metadata boxes. Fragmented/auxiliary-offset/item-offset variants
/// are rejected. A caller must discard output on error. Index limit is 32 MiB;
/// media bytes stream through a fixed copy buffer rather than loading the file.
pub fn fast_start<R: Read + Seek>(reader: &mut R, output: &mut impl Write) -> Result<()> {
    let end = reader.seek(SeekFrom::End(0))?;
    let mut spans = Vec::new();
    let mut at = 0;
    while at < end {
        if spans.len() >= 100_000 {
            return Err(invalid("too many top-level MP4 boxes"));
        }
        let span = header(reader, at, end)?;
        if matches!(&span.kind, b"moof" | b"sidx" | b"mfra" | b"meta") {
            return Err(invalid("MP4 relocation requires non-fragmented media"));
        }
        at += span.size;
        spans.push(span);
    }
    let movies: Vec<_> = spans
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == *b"moov")
        .map(|(i, _)| i)
        .collect();
    if movies.len() != 1 {
        return Err(invalid("MP4 relocation requires one moov"));
    }
    let movie = movies[0];
    let media = spans
        .iter()
        .position(|s| s.kind == *b"mdat")
        .ok_or_else(|| invalid("MP4 has no mdat"))?;
    if spans[movie].size > 32 << 20 {
        return Err(invalid("MP4 index exceeds relocation budget"));
    }
    let mut order: Vec<_> = (0..spans.len()).filter(|i| *i != movie).collect();
    let position = order.iter().position(|i| *i == media).unwrap();
    order.insert(position, movie);
    let mut locations = vec![0; spans.len()];
    let mut next = 0u64;
    for &index in &order {
        locations[index] = next;
        next += spans[index].size;
    }
    let mut moov = vec![0; spans[movie].size as usize];
    reader.seek(SeekFrom::Start(spans[movie].start))?;
    reader.read_exact(&mut moov)?;
    if moov[..4] == [0; 4] {
        moov[..4].copy_from_slice(&(spans[movie].size as u32).to_be_bytes());
    }
    patch(&mut moov, &spans, &locations, 0)?;
    for index in order {
        if index == movie {
            output.write_all(&moov)?;
        } else {
            reader.seek(SeekFrom::Start(spans[index].start))?;
            let copied = std::io::copy(&mut reader.take(spans[index].size), output)?;
            if copied != spans[index].size {
                return Err(invalid("MP4 input truncated during copy"));
            }
        }
    }
    Ok(())
}
