#[derive(Clone, Copy)]
pub struct Element {
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
/// Stand at `pos` on the way past it rather than by starting again. A walk moves
/// from one element to the next, and the bytes in between are the payload it has
/// just measured: reading them costs their own length, while a seek costs the
/// buffer behind the reader its whole capacity and the next read fetches the
/// same bytes back. Only a gap longer than any block the reader steps over, or a
/// move backwards, is a jump worth the seek.
const SKIP: u64 = 8 << 20;
fn goto<R: Read + Seek>(r: &mut R, pos: u64) -> Result<()> {
    let here = r.stream_position()?;
    if pos == here {
        return Ok(());
    }
    if pos > here && pos - here <= SKIP {
        let mut buf = [0u8; 1 << 13];
        let mut left = pos - here;
        while left > 0 {
            let want = (left as usize).min(buf.len());
            // A short read is the source answering less than asked, not an end:
            // no bytes at all means the item stopped before the element its own
            // size promised.
            let taken = r.read(&mut buf[..want])?;
            if taken == 0 {
                return Err(invalid("truncated EBML element"));
            }
            left -= taken as u64;
        }
        return Ok(());
    }
    r.seek(SeekFrom::Start(pos))?;
    Ok(())
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
/// A UTF-8 string element, without the trailing NUL byte some muxers write.
fn text<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Result<String> {
    Ok(String::from_utf8(bytes(r, e, max)?)
        .map_err(|_| invalid("invalid WebM text"))?
        .trim_end_matches('\0')
        .to_owned())
}
fn uint<R: Read + Seek>(r: &mut R, e: Element) -> Result<u64> {
    Ok(bytes(r, e, 8)?
        .into_iter()
        .fold(0, |v, b| (v << 8) | u64::from(b)))
}
fn sint<R: Read + Seek>(r: &mut R, e: Element) -> Result<i64> {
    let value = bytes(r, e, 8)?;
    let mut signed = if value.first().is_some_and(|byte| byte & 0x80 != 0) { -1i64 } else { 0 };
    for byte in value { signed = (signed << 8) | i64::from(byte); }
    Ok(signed)
}
/// Timestamps and rates are IEEE-754, written as either four or eight bytes.
fn float<R: Read + Seek>(r: &mut R, e: Element) -> Result<f64> {
    let value = bytes(r, e, 8)?;
    Ok(match value.as_slice() {
        [a, b, c, d] => f64::from(f32::from_be_bytes([*a, *b, *c, *d])),
        [a, b, c, d, e0, e1, e2, e3] => f64::from_be_bytes([*a, *b, *c, *d, *e0, *e1, *e2, *e3]),
        _ => return Err(invalid("invalid WebM float size")),
    })
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
        goto(r, at)?;
        let child = element(r, limit, count, max)?;
        at = end(child)?;
        out.push(child);
    }
    Ok(out)
}
