//! Owned Layer III header and side-information parsing. No spectral decoding.
use crate::{Result, invalid, mp3_hybrid::Window};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Mpeg1,
    Mpeg2,
    Mpeg25,
}
#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub version: Version,
    pub sample_rate: u32,
    pub channels: u16,
    pub mode: u8,
    pub mode_extension: u8,
    pub frame_bytes: usize,
    pub crc: bool,
    pub free: bool,
}
impl Header {
    pub fn granules(self) -> usize {
        if self.version == Version::Mpeg1 { 2 } else { 1 }
    }
    pub fn samples(self) -> usize {
        self.granules() * 576
    }
    pub fn side_bytes(self) -> usize {
        match (self.version, self.channels) {
            (Version::Mpeg1, 1) => 17,
            (Version::Mpeg1, _) => 32,
            (_, 1) => 9,
            _ => 17,
        }
    }
    pub fn parse(packet: &[u8]) -> Result<Self> {
        let h = u32::from_be_bytes(
            packet
                .get(..4)
                .ok_or_else(|| invalid("truncated MP3 header"))?
                .try_into()
                .unwrap(),
        );
        if h >> 21 != 0x7ff || (h >> 17) & 3 != 1 || h & 3 == 2 {
            return Err(invalid("invalid MP3 sync, layer or emphasis"));
        }
        let version = match (h >> 19) & 3 {
            3 => Version::Mpeg1,
            2 => Version::Mpeg2,
            0 => Version::Mpeg25,
            _ => return Err(invalid("reserved MP3 version")),
        };
        let rate_index = ((h >> 10) & 3) as usize;
        let index = ((h >> 12) & 15) as usize;
        if rate_index == 3 || index == 15 {
            return Err(invalid("reserved MP3 sample-rate or bitrate index"));
        }
        let sample_rate = [44100, 48000, 32000][rate_index]
            >> match version {
                Version::Mpeg1 => 0,
                Version::Mpeg2 => 1,
                Version::Mpeg25 => 2,
            };
        let rates = if version == Version::Mpeg1 {
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
            ]
        } else {
            [
                0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
            ]
        };
        let mode = ((h >> 6) & 3) as u8;
        let free = index == 0;
        let frame_bytes = if free {
            packet.len()
        } else {
            ((if version == Version::Mpeg1 {
                144000
            } else {
                72000
            }) * rates[index]
                / sample_rate
                + ((h >> 9) & 1)) as usize
        };
        Ok(Self {
            version,
            sample_rate,
            channels: if mode == 3 { 1 } else { 2 },
            mode,
            mode_extension: ((h >> 4) & 3) as u8,
            frame_bytes,
            crc: h & (1 << 16) == 0,
            free,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Granule {
    pub part2_3_bits: u16,
    pub big_values: u16,
    pub global_gain: u8,
    pub scalefac_compress: u16,
    pub window: Window,
    pub mixed: bool,
    pub tables: [u8; 3],
    pub subblock_gain: [u8; 3],
    pub regions: Option<(u8, u8)>,
    pub preflag: Option<bool>,
    pub scalefac_scale: bool,
    pub count1_table: bool,
}
impl Default for Granule {
    fn default() -> Self {
        Self {
            part2_3_bits: 0,
            big_values: 0,
            global_gain: 0,
            scalefac_compress: 0,
            window: Window::Normal,
            mixed: false,
            tables: [0; 3],
            subblock_gain: [0; 3],
            regions: None,
            preflag: None,
            scalefac_scale: false,
            count1_table: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SideInfo {
    pub main_data_begin: u16,
    pub scfsi: [[bool; 4]; 2],
    pub granules: [[Granule; 2]; 2],
}
pub struct Frame<'a> {
    pub header: Header,
    pub side: SideInfo,
    pub main_data: &'a [u8],
}
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u32> {
        if n > 32 || self.pos + n > self.data.len() * 8 {
            return Err(invalid("truncated MP3 side information"));
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(v)
    }
}
fn crc(bytes: impl Iterator<Item = u8>) -> u16 {
    let mut c = 65535u16;
    for byte in bytes {
        for i in (0..8).rev() {
            let feedback = (c >> 15) ^ u16::from((byte >> i) & 1);
            c <<= 1;
            if feedback != 0 {
                c ^= 0x8005;
            }
        }
    }
    c
}
fn checked_side(packet: &[u8]) -> Result<(Header, usize, &[u8])> {
    let header = Header::parse(packet)?;
    if packet.len() != header.frame_bytes {
        return Err(invalid("MP3 packet does not contain one complete frame"));
    }
    let begin = 4 + if header.crc { 2 } else { 0 };
    let end = begin + header.side_bytes();
    let side = packet
        .get(begin..end)
        .ok_or_else(|| invalid("truncated MP3 side information"))?;
    if header.crc {
        let expected = u16::from_be_bytes(packet[4..6].try_into().unwrap());
        if crc(packet[2..4].iter().copied().chain(side.iter().copied())) != expected {
            return Err(invalid("MP3 side information CRC mismatch"));
        }
    }
    Ok((header, end, side))
}
impl<'a> Frame<'a> {
    // Historical protected Info tags overlap the final two side bytes. Only
    // coded lengths and big-values identify actual audio; spectral table fields
    // in those metadata bytes are not audio declarations.
    pub(crate) fn parse_info(packet: &'a [u8]) -> Result<Option<Self>> {
        if crate::mp3_gapless::tag_offset(packet)?.is_none() {
            return Ok(None);
        }
        let (header, end, side) = checked_side(packet)?;
        let mpeg1 = header.version == Version::Mpeg1;
        let channels = usize::from(header.channels);
        let mut bits = Bits { data: side, pos: 0 };
        let back = bits.read(if mpeg1 { 9 } else { 8 })? as u16;
        bits.read(if mpeg1 {
            if channels == 1 { 5 } else { 3 }
        } else {
            if channels == 1 { 1 } else { 2 }
        })?;
        if mpeg1 {
            bits.read(channels * 4)?;
        }
        for _ in 0..header.granules() * channels {
            let length = bits.read(12)?;
            let big = bits.read(9)?;
            if length != 0 || big != 0 {
                return Ok(None);
            }
            bits.read(32)?;
            bits.read(if mpeg1 { 6 } else { 10 })?;
        }
        Ok(Some(Self {
            header,
            side: SideInfo {
                main_data_begin: back,
                scfsi: [[false; 4]; 2],
                granules: [[Granule::default(); 2]; 2],
            },
            main_data: &packet[end..],
        }))
    }
    pub fn parse(packet: &'a [u8]) -> Result<Self> {
        let (header, end, side) = checked_side(packet)?;
        let mut b = Bits { data: side, pos: 0 };
        let mpeg1 = header.version == Version::Mpeg1;
        let channels = usize::from(header.channels);
        let main_data_begin = b.read(if mpeg1 { 9 } else { 8 })? as u16;
        b.read(if mpeg1 {
            if channels == 1 { 5 } else { 3 }
        } else if channels == 1 {
            1
        } else {
            2
        })?;
        let mut info = SideInfo {
            main_data_begin,
            scfsi: [[false; 4]; 2],
            granules: [[Granule::default(); 2]; 2],
        };
        if mpeg1 {
            for channel in info.scfsi.iter_mut().take(channels) {
                for bit in channel {
                    *bit = b.read(1)? != 0;
                }
            }
        }
        for group in info.granules.iter_mut().take(header.granules()) {
            for g in group.iter_mut().take(channels) {
                g.part2_3_bits = b.read(12)? as u16;
                g.big_values = b.read(9)? as u16;
                if g.big_values > 288 {
                    return Err(invalid("MP3 big_values exceeds 288 pairs"));
                }
                g.global_gain = b.read(8)? as u8;
                g.scalefac_compress = b.read(if mpeg1 { 4 } else { 9 })? as u16;
                if b.read(1)? != 0 {
                    g.window = match b.read(2)? {
                        1 => Window::Start,
                        2 => Window::Short,
                        3 => Window::Stop,
                        _ => {
                            return Err(invalid(
                                "MP3 switched window cannot have normal block type",
                            ));
                        }
                    };
                    g.mixed = b.read(1)? != 0;
                    for table in &mut g.tables[..2] {
                        *table = b.read(5)? as u8;
                    }
                    for gain in &mut g.subblock_gain {
                        *gain = b.read(3)? as u8;
                    }
                } else {
                    for table in &mut g.tables {
                        *table = b.read(5)? as u8;
                    }
                    let a = b.read(4)? as u8;
                    let c = b.read(3)? as u8;
                    if a + c > 20 {
                        return Err(invalid("MP3 region counts exceed spectral bands"));
                    }
                    g.regions = Some((a, c));
                }
                if g.tables.iter().any(|&t| t == 4 || t == 14) {
                    return Err(invalid("reserved MP3 Huffman table"));
                }
                if mpeg1 {
                    g.preflag = Some(b.read(1)? != 0);
                }
                g.scalefac_scale = b.read(1)? != 0;
                g.count1_table = b.read(1)? != 0;
            }
        }
        if b.pos != side.len() * 8 {
            return Err(invalid("MP3 side information length mismatch"));
        }
        Ok(Self {
            header,
            side: info,
            main_data: &packet[end..],
        })
    }
}
/// Transactional main-data assembly; only committed payload bytes enter history.
#[derive(Clone, Default)]
pub struct Reservoir {
    history: Vec<u8>,
    generation: u64,
}
pub struct MainData {
    pub bytes: Vec<u8>,
    pub ranges: Vec<std::ops::Range<usize>>,
    next_history: Vec<u8>,
    generation: u64,
}
impl Reservoir {
    pub fn reset(&mut self) {
        self.history.clear();
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn prepare(&self, frame: &Frame<'_>) -> Result<MainData> {
        if !(1..=2).contains(&frame.header.channels) {
            return Err(invalid("invalid MP3 reservoir channel count"));
        }
        let back = usize::from(frame.side.main_data_begin);
        let limit = if frame.header.version == Version::Mpeg1 {
            511
        } else {
            255
        };
        if back > limit || back > self.history.len() {
            return Err(invalid("MP3 reservoir history missing for main_data_begin"));
        }
        let mut total = 0;
        let mut ranges = Vec::new();
        for group in frame.side.granules.iter().take(frame.header.granules()) {
            for g in group.iter().take(usize::from(frame.header.channels)) {
                if g.part2_3_bits > 4095 {
                    return Err(invalid("MP3 granule bit length exceeds 4095"));
                }
                let end = total + usize::from(g.part2_3_bits);
                ranges.push(total..end);
                total = end;
            }
        }
        let available = back
            .checked_add(frame.main_data.len())
            .ok_or_else(|| invalid("MP3 main-data length overflow"))?;
        let needed = total.div_ceil(8);
        if needed > available {
            return Err(invalid("MP3 granule bit ranges exceed available main data"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(needed)
            .map_err(|_| invalid("MP3 main-data allocation failed"))?;
        bytes.extend(
            self.history[self.history.len() - back..]
                .iter()
                .chain(frame.main_data)
                .take(needed)
                .copied(),
        );
        let held = self
            .history
            .len()
            .checked_add(frame.main_data.len())
            .ok_or_else(|| invalid("MP3 reservoir length overflow"))?;
        let keep = held.min(511);
        let mut next_history = Vec::new();
        next_history
            .try_reserve_exact(keep)
            .map_err(|_| invalid("MP3 reservoir allocation failed"))?;
        next_history.extend(
            self.history
                .iter()
                .chain(frame.main_data)
                .skip(held - keep)
                .copied(),
        );
        Ok(MainData {
            bytes,
            ranges,
            next_history,
            generation: self.generation,
        })
    }
    pub fn commit(&mut self, data: MainData) -> Result<()> {
        if data.generation != self.generation {
            return Err(invalid("stale MP3 reservoir transaction"));
        }
        self.history = data.next_history;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
}
