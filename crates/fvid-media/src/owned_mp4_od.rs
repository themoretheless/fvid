//! Bounded MP4 file-reference Object Descriptor commands (ISO 14496-1/14).
//! OD state is separate from codec state; an AU applies atomically.
#![forbid(unsafe_code)]
type Result<T> = std::result::Result<T, String>;
fn bad(s: &str) -> String {
    s.into()
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub id: u16,
    pub tracks: Vec<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub objects: Vec<Object>,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub pts: i64,
    pub state: State,
}
#[derive(Clone, Debug)]
pub struct Timeline {
    pub track_id: u32,
    pub timescale: u32,
    pub references: Vec<u32>,
    pub events: Vec<Event>,
}
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
}
impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            end: data.len() * 8,
        }
    }
    fn left(&self) -> usize {
        self.end - self.at
    }
    fn read(&mut self, n: usize) -> Result<u32> {
        if n > 32 || n > self.left() {
            return Err(bad("truncated MP4 OD field"));
        }
        let mut out = 0;
        for _ in 0..n {
            out = out << 1 | u32::from(self.data[self.at / 8] >> (7 - self.at % 8) & 1);
            self.at += 1;
        }
        Ok(out)
    }
    fn record(&mut self) -> Result<(u8, Self)> {
        let tag = self.read(8)? as u8;
        let mut len = 0usize;
        for i in 0..4 {
            let v = self.read(8)?;
            len = (len << 7) | (v as usize & 127);
            if v & 128 == 0 {
                let bits = len
                    .checked_mul(8)
                    .filter(|&n| n <= self.left())
                    .ok_or_else(|| bad("MP4 OD descriptor exceeds boundary"))?;
                let child = Self {
                    data: self.data,
                    at: self.at,
                    end: self.at + bits,
                };
                self.at += bits;
                return Ok((tag, child));
            }
            if i == 3 {
                return Err(bad("MP4 OD descriptor length exceeds four bytes"));
            }
        }
        unreachable!()
    }
    fn end(&self) -> Result<()> {
        if self.left() != 0 {
            Err(bad("unconsumed MP4 OD descriptor bits"))
        } else {
            Ok(())
        }
    }
    fn align(&mut self) -> Result<()> {
        if self.left() >= 8 || self.read(self.left())? != 0 {
            Err(bad("nonzero MP4 OD command alignment"))
        } else {
            Ok(())
        }
    }
}
fn refs(b: &mut Bits<'_>, map: &[u32], aligned: bool) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    while b.left() > (if aligned { 0 } else { 7 }) {
        let (tag, mut body) = b.record()?;
        if tag != 15 {
            return Err(bad("MP4 OD stream requires ES_ID_Ref descriptors"));
        }
        let index = body.read(16)? as usize;
        body.end()?;
        let id = *index
            .checked_sub(1)
            .and_then(|i| map.get(i))
            .ok_or_else(|| bad("MP4 OD reference index out of range"))?;
        if id == 0 || out.contains(&id) {
            return Err(bad("duplicate or zero MP4 OD ES reference"));
        }
        out.try_reserve_exact(1)
            .map_err(|_| bad("MP4 OD reference allocation failed"))?;
        out.push(id);
    }
    if !aligned {
        b.align()?;
    }
    Ok(out)
}
/// File-only ES_ID_Inc descriptors identify the streams reachable from iods.
pub fn initial_tracks(data: &[u8], max_tracks: usize) -> Result<Vec<u32>> {
    if data.get(..4) != Some(&[0, 0, 0, 0]) {
        return Err(bad("unsupported MP4 iods version or flags"));
    }
    let mut b = Bits::new(&data[4..]);
    let (tag, mut root) = b.record()?;
    b.end()?;
    if tag != 16 && tag != 17 {
        return Err(bad("MP4 iods requires file object descriptor"));
    }
    let id = root.read(10)?;
    let url = root.read(1)?;
    if id == 0 || url != 0 {
        return Err(bad("unsupported MP4 OD identifier or URL"));
    }
    if tag == 16 {
        root.read(1)?;
        if root.read(4)? != 15 {
            return Err(bad("invalid MP4 IOD reserved bits"));
        }
        for _ in 0..5 {
            root.read(8)?;
        }
    } else if root.read(5)? != 31 {
        return Err(bad("invalid MP4 OD reserved bits"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(max_tracks.min(root.left() / 48))
        .map_err(|_| bad("MP4 OD root allocation failed"))?;
    while root.left() != 0 {
        if out.len() == max_tracks {
            return Err(bad("MP4 initial OD exceeds track limit"));
        }
        let (tag, mut body) = root.record()?;
        if tag != 14 {
            return Err(bad("unsupported MP4 initial object descriptor child"));
        }
        let track = body.read(32)?;
        body.end()?;
        if track == 0 || out.contains(&track) {
            return Err(bad("duplicate or zero MP4 initial OD track"));
        }
        out.try_reserve_exact(1)
            .map_err(|_| bad("MP4 OD root allocation failed"))?;
        out.push(track);
    }
    Ok(out)
}
/// A self-contained MPEG-4 Systems OD stream, not an AAC DecoderSpecificInfo.
pub fn validate_setup(data: &[u8]) -> Result<()> {
    if data.get(..4) != Some(&[0, 0, 0, 0]) {
        return Err(bad("unsupported MP4 OD esds version or flags"));
    }
    let mut b = Bits::new(&data[4..]);
    let (tag, mut es) = b.record()?;
    b.end()?;
    if tag != 3 || es.read(16)? != 0 || es.read(3)? != 0 {
        return Err(bad("MP4 OD stream requires canonical ES binding"));
    }
    es.read(5)?;
    let (mut decoder, mut sl) = (false, false);
    while es.left() != 0 {
        let (tag, mut body) = es.record()?;
        match tag {
            4 => {
                if decoder {
                    return Err(bad("duplicate MP4 OD decoder config"));
                }
                decoder = true;
                if !matches!(body.read(8)?, 1 | 2)
                    || body.read(6)? != 1
                    || body.read(1)? != 0
                    || body.read(1)? != 1
                {
                    return Err(bad("unsupported MP4 OD decoder config"));
                }
                body.read(24)?;
                body.read(32)?;
                body.read(32)?;
                body.end()?;
            }
            6 => {
                if sl || body.read(8)? != 2 {
                    return Err(bad("MP4 OD requires self-contained SL configuration 2"));
                }
                sl = true;
                body.end()?;
            }
            _ => return Err(bad("unsupported MP4 OD ES descriptor child")),
        }
    }
    if !decoder || !sl {
        return Err(bad("incomplete MP4 OD stream setup"));
    }
    Ok(())
}
impl State {
    pub fn heap_bytes(&self) -> Result<usize> {
        let mut n = self
            .objects
            .capacity()
            .checked_mul(std::mem::size_of::<Object>())
            .ok_or_else(|| bad("MP4 OD allocation size overflow"))?;
        for o in &self.objects {
            n = n
                .checked_add(
                    o.tracks
                        .capacity()
                        .checked_mul(4)
                        .ok_or_else(|| bad("MP4 OD allocation size overflow"))?,
                )
                .ok_or_else(|| bad("MP4 OD allocation size overflow"))?;
        }
        Ok(n)
    }
    pub fn apply(&mut self, data: &[u8], map: &[u32]) -> Result<()> {
        let mut next = self.clone();
        let mut au = Bits::new(data);
        while au.left() != 0 {
            let (tag, mut cmd) = au.record()?;
            match tag {
                1 => {
                    let mut seen = Vec::new();
                    while cmd.left() != 0 {
                        let (tag, mut object) = cmd.record()?;
                        if tag != 17 {
                            return Err(bad("MP4 ODUpdate requires file object descriptors"));
                        }
                        let id = object.read(10)? as u16;
                        if id == 0 || object.read(1)? != 0 || object.read(5)? != 31 {
                            return Err(bad("unsupported MP4 OD identifier, URL or reserved bits"));
                        }
                        if seen.contains(&id) {
                            return Err(bad("duplicate object in MP4 ODUpdate"));
                        }
                        seen.push(id);
                        let tracks = refs(&mut object, map, true)?;
                        let entry = Object { id, tracks };
                        if let Some(old) = next.objects.iter_mut().find(|o| o.id == id) {
                            *old = entry;
                        } else {
                            next.objects
                                .try_reserve_exact(1)
                                .map_err(|_| bad("MP4 OD object allocation failed"))?;
                            next.objects.push(entry);
                        }
                    }
                }
                2 => {
                    while cmd.left() >= 10 {
                        let id = cmd.read(10)? as u16;
                        if id == 0 {
                            return Err(bad("zero MP4 OD removal identifier"));
                        }
                        next.objects.retain(|o| o.id != id);
                    }
                    cmd.align()?;
                }
                3 => {
                    let id = cmd.read(10)? as u16;
                    let add = refs(&mut cmd, map, false)?;
                    let object = next
                        .objects
                        .iter_mut()
                        .find(|o| o.id == id)
                        .ok_or_else(|| bad("MP4 ESDUpdate targets unknown object"))?;
                    for track in add {
                        if !object.tracks.contains(&track) {
                            object
                                .tracks
                                .try_reserve_exact(1)
                                .map_err(|_| bad("MP4 OD reference allocation failed"))?;
                            object.tracks.push(track);
                        }
                    }
                }
                7 => {
                    let id = cmd.read(10)? as u16;
                    if cmd.read(6)? != 0 || !cmd.left().is_multiple_of(16) {
                        return Err(bad("invalid MP4 ESRemoveRef alignment"));
                    }
                    let object = next
                        .objects
                        .iter_mut()
                        .find(|o| o.id == id)
                        .ok_or_else(|| bad("MP4 ESRemoveRef targets unknown object"))?;
                    while cmd.left() != 0 {
                        let index = cmd.read(16)? as usize;
                        let track = *index
                            .checked_sub(1)
                            .and_then(|i| map.get(i))
                            .ok_or_else(|| bad("MP4 OD reference index out of range"))?;
                        object.tracks.retain(|&t| t != track);
                    }
                }
                _ => return Err(bad("unsupported MP4 OD command")),
            }
        }
        *self = next;
        Ok(())
    }
    pub fn prefix(&self, chain: &[u32]) -> Result<usize> {
        let root = *chain
            .first()
            .ok_or_else(|| bad("empty MP4 OD programme chain"))?;
        let mut selected = self.objects.iter().filter(|o| o.tracks.contains(&root));
        let object = selected
            .next()
            .ok_or_else(|| bad("MP4 OD does not activate selected root"))?;
        if selected.next().is_some() {
            return Err(bad("MP4 OD root belongs to multiple objects"));
        }
        let width = chain
            .iter()
            .take_while(|id| object.tracks.contains(id))
            .count();
        if chain[width..].iter().any(|id| object.tracks.contains(id)) {
            return Err(bad("EP1 MP4 OD active dependency chain has a gap"));
        }
        Ok(width)
    }
}
impl Timeline {
    pub fn heap_bytes(&self) -> Result<usize> {
        let mut n = self
            .references
            .capacity()
            .checked_mul(4)
            .and_then(|v| {
                self.events
                    .capacity()
                    .checked_mul(std::mem::size_of::<Event>())
                    .and_then(|e| v.checked_add(e))
            })
            .ok_or_else(|| bad("MP4 OD allocation size overflow"))?;
        for e in &self.events {
            n = n
                .checked_add(e.state.heap_bytes()?)
                .ok_or_else(|| bad("MP4 OD allocation size overflow"))?;
        }
        Ok(n)
    }
    pub fn prefix(&self, chain: &[u32], pts: i64, scale: u32) -> Result<usize> {
        if scale == 0 || self.timescale == 0 {
            return Err(bad("zero MP4 OD timescale"));
        }
        self.prefix_units(
            chain,
            i128::from(pts) * i128::from(self.timescale),
            i128::from(scale),
        )
    }
    /// Exact common-clock coordinates, including file edit-list offsets.
    pub fn prefix_units(&self, chain: &[u32], at: i128, event_unit: i128) -> Result<usize> {
        if event_unit <= 0 {
            return Err(bad("zero MP4 OD timescale"));
        }
        for event in self.events.iter().rev() {
            let time = i128::from(event.pts)
                .checked_mul(event_unit)
                .ok_or_else(|| bad("MP4 OD edit clock overflow"))?;
            if time <= at {
                return event.state.prefix(chain);
            }
        }
        Err(bad("MP4 OD has no initial event at source time"))
    }
}
