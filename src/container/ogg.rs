//! An Ogg bitstream read as far as its own framing goes: the pages a file is
//! built from, and the logical packets reassembled out of them.
//!
//! A page states nothing about what it carries. Its header gives a serial number,
//! a position on the timeline of whatever the stream turns out to be, and a table
//! of segment lengths that *is* the packet boundary: a packet is the run of
//! segments up to and including one shorter than 255, and a run still at 255 when
//! the page ends is unfinished and continues on the next page of the same serial.
//! So the only container-level facts in an Ogg file are `OggS`, a checksum and a
//! lacing table; what a stream *is* comes from the first bytes of its first
//! packet, which is why [`Coding`] is read out of a magic number rather than out
//! of a field.
//!
//! Four of the framing's claims are checked instead of trusted, because each is a
//! number the file can get wrong in a way a reader cannot recover from: the
//! page's CRC-32, the segment table's total against the bytes the file holds, the
//! continuation flag against the packet left open on the serial, and a stream-end
//! flag that arrives while a packet is still being written. A page that fails one
//! of these is refused, not worked around: the reader has no way to guess where a
//! boundary it was told about actually was.
//!
//! Chaining - one bitstream ending and another of the same coding beginning, in
//! file order, as a gapless album or a concatenated pair of programs is written -
//! is what the serial and the two flags describe, so chains are kept apart and in
//! the order their first pages appear. A file that interleaves two live streams
//! keeps them apart by serial too, and is read the same way.

use crate::{Result, invalid};

/// What a limit guards: how much the reader may take in, and how many packets it
/// may agree to list across all of the file's streams.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Most packets the whole file may hold.
    pub packets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 128 << 20,
            packets: 1 << 20,
        }
    }
}

/// The magic a logical bitstream opens with. Ogg names no codec, so this is the
/// first packet's own claim about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    /// `\x01vorbis`, whose audio packets follow three header packets.
    Vorbis,
    /// `OpusHead`, whose granules count 48 kHz samples and whose pre-skip is a
    /// field of its own.
    Opus,
    /// `\x80theora`, a video bitstream with three header packets too.
    Theora,
    /// `\x7fFLAC`, whose packets are FLAC frames with the metadata blocks ahead.
    Flac,
    /// `Speex   `, the narrowband codec shipped in Ogg.
    Speex,
    /// `fisb`: the index tables a muxer writes beside its streams, which carry no
    /// media at all.
    Skeleton,
    /// A bitstream whose opening bytes name nothing on this list.
    Other,
}

impl Coding {
    /// What the first bytes of a bitstream's first packet say it is.
    fn of(first: &[u8]) -> Self {
        if first.starts_with(b"\x01vorbis") {
            Self::Vorbis
        } else if first.starts_with(b"OpusHead") {
            Self::Opus
        } else if first.starts_with(b"\x80theora") {
            Self::Theora
        } else if first.starts_with(b"\x7fFLAC") {
            Self::Flac
        } else if first.starts_with(b"Speex   ") {
            Self::Speex
        } else if first.starts_with(b"fisb") {
            Self::Skeleton
        } else {
            Self::Other
        }
    }

    /// How to name this bitstream in a refusal. Where the list above has the
    /// coding, its own word is the useful one; where it does not, the bytes that
    /// decided that are what a reader of the message needs.
    pub fn describe(&self, opening: &[u8]) -> String {
        match self {
            Self::Vorbis => "vorbis",
            Self::Opus => "OpusHead",
            Self::Theora => "theora",
            Self::Flac => "FLAC in Ogg",
            Self::Speex => "Speex",
            Self::Skeleton => "skeleton",
            Self::Other => {
                let head: String = opening
                    .iter()
                    .take(8)
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                return format!("a bitstream opening {head}");
            }
        }
        .to_owned()
    }
}

/// One logical packet, and what the page that finished it stated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub data: Vec<u8>,
    /// The position the page carrying the end of this packet declared, and only
    /// for the last packet the page finishes: a page's position is the position of
    /// its last complete packet. `-1`, which is what a writer puts where it states
    /// nothing, is kept as no position at all.
    pub end_granule: Option<i64>,
}

/// One run of one bitstream: from the page that began it to the page that ended
/// it, with every packet the pages reassemble into.
#[derive(Clone, Debug)]
pub struct Chain {
    pub serial: u32,
    /// What the first packet claimed. A chain whose first packet never completes
    /// inside the file is refused before this is ever read.
    pub coding: Coding,
    pub packets: Vec<Packet>,
    /// Whether a page of this serial said the stream was over.
    pub ended: bool,
}

/// A file's pages, walked in order, and the chains they reassemble into.
#[derive(Clone, Debug)]
pub struct Ogg {
    chains: Vec<Chain>,
    pages: usize,
}

/// A serial with a chain still being written, and the bytes of the packet it is
/// in the middle of.
struct Open {
    serial: u32,
    chain: usize,
    pending: Vec<u8>,
}

impl Ogg {
    /// Walk every page of a file and rebuild the packets out of their segment
    /// tables.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        let mut chains: Vec<Chain> = Vec::new();
        let mut open: Vec<Open> = Vec::new();
        let mut listed = 0usize;
        let mut at = 0usize;
        let mut pages = 0usize;
        while at < bytes.len() {
            if bytes.get(at..at + 4) != Some(b"OggS") {
                return Err(invalid(&format!(
                    "Ogg page {pages} at byte {at} does not begin with `OggS`"
                )));
            }
            let head = bytes.get(at..at + 27).ok_or_else(|| {
                invalid(&format!(
                    "Ogg page {pages} at byte {at} is cut off inside its own header"
                ))
            })?;
            // Every writer of every Ogg file yet made has used version 0, so any
            // other number means the framing below is not the framing meant.
            if head[4] != 0 {
                return Err(invalid(&format!(
                    "Ogg stream format version {} is not the framing this reader walks",
                    head[4]
                )));
            }
            let (flags, counted) = (head[5], usize::from(head[26]));
            let table = bytes.get(at + 27..at + 27 + counted).ok_or_else(|| {
                invalid(&format!(
                    "Ogg page {pages} at byte {at} lists {counted} segments past the end of the file"
                ))
            })?;
            let body: usize = table.iter().map(|length| usize::from(*length)).sum();
            let end = at + 27 + counted + body;
            let segments = bytes.get(at + 27 + counted..end).ok_or_else(|| {
                invalid(&format!(
                    "Ogg page {pages} states {body} bytes of body and the file ends at {}",
                    bytes.len()
                ))
            })?;
            let stored = u32::from_le_bytes(head[22..26].try_into().expect("four checksum bytes"));
            let computed = checksum(at, end, bytes);
            if stored != computed {
                return Err(invalid(&format!(
                    "Ogg page {pages} at byte {at} states checksum {stored:#010x} over its own bytes, which add up to {computed:#010x}"
                )));
            }
            pages += 1;
            let serial = u32::from_le_bytes(head[14..18].try_into().expect("four serial bytes"));
            let granule = i64::from_le_bytes(head[6..14].try_into().expect("eight granule bytes"));
            let position = match open.iter().position(|at| at.serial == serial) {
                Some(position) => {
                    if flags & BEGINS != 0 {
                        return Err(invalid(&format!(
                            "Ogg page {pages} continues bitstream {serial:#x} and begins it again in the same place"
                        )));
                    }
                    position
                }
                None => {
                    if flags & CONTINUES != 0 {
                        return Err(invalid(&format!(
                            "Ogg page {pages} continues a packet of bitstream {serial:#x} that never began"
                        )));
                    }
                    if flags & BEGINS == 0 {
                        return Err(invalid(&format!(
                            "bitstream {serial:#x} begins at byte {at} without saying it begins one"
                        )));
                    }
                    open.push(Open {
                        serial,
                        chain: chains.len(),
                        pending: Vec::new(),
                    });
                    chains.push(Chain {
                        serial,
                        coding: Coding::Other,
                        packets: Vec::new(),
                        ended: false,
                    });
                    open.len() - 1
                }
            };
            // The segment table is the packet boundary: a run of 255s keeps going,
            // and the first byte under 255 ends the packet with as many more bytes
            // as it says. A run that reaches the end of the page unfinished is the
            // continued packet the page's first flag told about.
            let mut runs: Vec<(usize, usize, bool)> = Vec::new();
            let (mut start, mut length) = (0usize, 0usize);
            for length_at in table {
                length += usize::from(*length_at);
                if *length_at < 255 {
                    runs.push((start, length, false));
                    start += length;
                    length = 0;
                }
            }
            if length > 0 {
                runs.push((start, length, true));
            }
            let at_open = &mut open[position];
            // Only the last packet the page finishes carries its position: a page
            // numbers itself by where that packet ends, and every packet before it
            // on the page ends sooner.
            let finishes = runs.len() - usize::from(runs.last().is_some_and(|(_, _, open)| *open));
            for (number, (offset, length, unfinished)) in runs.into_iter().enumerate() {
                let piece = &segments[offset..offset + length];
                let data = if number == 0 && flags & CONTINUES != 0 {
                    at_open.pending.extend_from_slice(piece);
                    if unfinished {
                        continue;
                    }
                    std::mem::take(&mut at_open.pending)
                } else if unfinished {
                    at_open.pending.extend_from_slice(piece);
                    continue;
                } else {
                    piece.to_vec()
                };
                listed += 1;
                if listed > limits.packets {
                    return Err(invalid(&format!(
                        "file is over the {} packet limit",
                        limits.packets
                    )));
                }
                let chain = &mut chains[at_open.chain];
                if chain.packets.is_empty() {
                    chain.coding = Coding::of(&data);
                }
                let stated = number + 1 == finishes;
                chain.packets.push(Packet {
                    data,
                    end_granule: stated.then(|| (granule >= 0).then_some(granule)).flatten(),
                });
            }
            if flags & ENDS != 0 {
                if !at_open.pending.is_empty() {
                    return Err(invalid(&format!(
                        "bitstream {serial:#x} ends at byte {at} with a packet still open"
                    )));
                }
                chains[at_open.chain].ended = true;
                open.remove(position);
            }
            at = end;
        }
        if chains.is_empty() {
            return Err(invalid("Ogg file holds no bitstream"));
        }
        // A stream that never said it was over is a truncated file rather than a
        // refusal: everything up to the cut is there and states its own position.
        Ok(Self { chains, pages })
    }

    /// The chains of the file, in the order their first pages appear.
    pub fn chains(&self) -> &[Chain] {
        &self.chains
    }

    /// How many pages the walk read.
    pub fn pages(&self) -> usize {
        self.pages
    }
}

/// A page says it continues the packet the last page of the same serial left
/// unfinished.
const CONTINUES: u8 = 0x01;
/// ...and this one is where a bitstream begins.
const BEGINS: u8 = 0x02;
/// ...and no packet of this bitstream follows.
const ENDS: u8 = 0x04;

/// The Ogg checksum: CRC-32 with the polynomial the format fixes, no reflection,
/// no initial vector and no final inversion. A page is summed over its own bytes
/// with its four checksum bytes replaced by zeroes, which is why the span is
/// walked in three parts.
pub(crate) fn checksum(from: usize, until: usize, bytes: &[u8]) -> u32 {
    let mut crc = 0u32;
    crc = crc_run(crc, &bytes[from..from + 22]);
    crc = crc_run(crc, &[0u8; 4]);
    crc_run(crc, &bytes[from + 26..until])
}

fn crc_run(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc = (crc << 8) ^ TABLE[(((crc >> 24) ^ u32::from(*byte)) & 0xff) as usize];
    }
    crc
}

/// The 256 words the CRC above steps through, with the Ogg polynomial.
const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut word = (index as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            word = if word & 0x8000_0000 != 0 {
                (word << 1) ^ 0x04c1_1db7
            } else {
                word << 1
            };
            bit += 1;
        }
        table[index] = word;
        index += 1;
    }
    table
};

/// Re-seal every page of a file whose bytes a test has already changed, so that
/// the refusal it proves is the one about the field it broke rather than the one
/// about the checksum. A doctored copy that trips the CRC first has proved only
/// that the CRC fires.
#[cfg(all(test, feature = "player"))]
pub(crate) fn rechecksum(bytes: &mut [u8]) {
    let mut at = 0usize;
    while bytes.get(at..at + 4) == Some(b"OggS") {
        let Some(head) = bytes.get(at..at + 27) else {
            return;
        };
        let counted = usize::from(head[26]);
        let Some(table) = bytes.get(at + 27..at + 27 + counted) else {
            return;
        };
        let body: usize = table.iter().map(|length| usize::from(*length)).sum();
        let end = at + 27 + counted + body;
        if bytes.get(end).is_none() {
            return;
        }
        let sum = checksum(at, end, bytes);
        bytes[at + 22..at + 26].copy_from_slice(&sum.to_le_bytes());
        at = end;
    }
}

#[cfg(test)]
mod tests {
    use super::{Coding, Limits, Ogg};

    /// One page of one bitstream, lacing every packet given: 255s while the packet
    /// runs, then how many bytes the last segment of it holds - zero where the
    /// packet ends on a page boundary, which is what keeps the next packet from
    /// looking like its continuation.
    fn page(serial: u32, number: u32, granule: i64, flags: u8, packets: &[Vec<u8>]) -> Vec<u8> {
        let mut table = Vec::new();
        let mut body = Vec::new();
        for packet in packets {
            body.extend_from_slice(packet);
            let mut rest = packet.len();
            while rest >= 255 {
                table.push(255u8);
                rest -= 255;
            }
            table.push(rest as u8);
        }
        sealed(serial, number, granule, flags, &table, &body)
    }

    /// A page whose segment table and body are stated exactly. Where the lacing
    /// helper above always closes the packet it is handed, this is how a test
    /// writes the framing a writer left unfinished.
    fn sealed(
        serial: u32,
        number: u32,
        granule: i64,
        flags: u8,
        table: &[u8],
        body: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"OggS");
        out.push(0);
        out.push(flags);
        out.extend_from_slice(&granule.to_le_bytes());
        out.extend_from_slice(&serial.to_le_bytes());
        out.extend_from_slice(&number.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.push(table.len() as u8);
        out.extend_from_slice(table);
        out.extend_from_slice(body);
        let sum = super::checksum(0, out.len(), &out);
        out[22..26].copy_from_slice(&sum.to_le_bytes());
        out
    }

    fn parse(bytes: &[u8]) -> Ogg {
        Ogg::parse(bytes, &Limits::default()).expect("parses")
    }

    #[test]
    fn a_packet_spanning_pages_is_reassembled_from_its_segments() {
        // 255 bytes of one packet on the first page, the rest on the second, then a
        // short packet of its own: the lacing rule that lets a Vorbis setup header
        // be longer than a page.
        let first = page(1, 0, -1, 2, &[vec![7u8; 300]]);
        let second = page(1, 1, 42, 4, &[vec![9u8; 12]]);
        let ogg = parse(&[first, second].concat());
        assert_eq!(ogg.chains().len(), 1);
        let chain = &ogg.chains()[0];
        assert_eq!(chain.packets.len(), 2);
        assert_eq!(chain.packets[0].data, vec![7u8; 300]);
        assert_eq!(chain.packets[1].data, vec![9u8; 12]);
        // The position belongs to the packet the page finished.
        assert_eq!(chain.packets[0].end_granule, None);
        assert_eq!(chain.packets[1].end_granule, Some(42));
        assert!(chain.ended);
        assert_eq!(ogg.pages(), 2);
    }

    #[test]
    fn two_bitstreams_interleaved_page_by_page_stay_apart() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(1, 0, 0, 2, &[b"\x01vorbis\x00".to_vec()]));
        bytes.extend_from_slice(&page(2, 0, 0, 2, &[b"OpusHead\x00".to_vec()]));
        bytes.extend_from_slice(&page(1, 1, 10, 4, &[vec![1]]));
        bytes.extend_from_slice(&page(2, 1, 20, 4, &[vec![2]]));
        let ogg = parse(&bytes);
        assert_eq!(ogg.chains().len(), 2);
        assert_eq!(ogg.chains()[0].serial, 1);
        assert_eq!(ogg.chains()[0].coding, Coding::Vorbis);
        assert_eq!(ogg.chains()[1].coding, Coding::Opus);
        assert_eq!(
            ogg.chains()
                .iter()
                .map(|chain| chain
                    .packets
                    .iter()
                    .map(|p| p.data.clone())
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![
                vec![b"\x01vorbis\x00".to_vec(), vec![1]],
                vec![b"OpusHead\x00".to_vec(), vec![2]]
            ]
        );
    }

    #[test]
    fn a_chain_is_named_by_the_packet_it_opens_with() {
        let named = Coding::of;
        assert_eq!(named(b"\x01vorbis"), Coding::Vorbis);
        assert_eq!(named(b"OpusHead"), Coding::Opus);
        assert_eq!(named(b"\x80theora"), Coding::Theora);
        assert_eq!(named(b"\x7fFLAC\x01\x00"), Coding::Flac);
        assert_eq!(named(b"Speex   "), Coding::Speex);
        assert_eq!(named(b"fisb\x00"), Coding::Skeleton);
        // The four bytes are the whole magic, so a sibling of the index tables that
        // shares their prefix is an unknown bitstream rather than one named.
        assert_eq!(named(b"fishtail"), Coding::Other);
        assert_eq!(named(b"\x00\x01"), Coding::Other);
        // A refusal has to name what it turned down.
        assert_eq!(Coding::Opus.describe(b"OpusHead"), "OpusHead");
        assert_eq!(
            Coding::Other.describe(b"\x11\x22"),
            "a bitstream opening 1122"
        );
    }

    /// The checksum is the framing's own statement about its bytes, so a page that
    /// lies about them is not read further.
    #[test]
    fn a_page_failing_its_own_checksum_is_refused() {
        let mut bytes = page(1, 0, 0, 2, &[vec![3u8; 8]]);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let error = Ogg::parse(&bytes, &Limits::default())
            .err()
            .expect("a corrupted page is not a page")
            .to_string();
        assert!(error.contains("checksum"), "{error}");
    }

    #[test]
    fn framing_that_does_not_add_up_is_refused_rather_than_guessed() {
        // A page whose segment table reaches past the bytes the file holds.
        let mut cut = page(1, 0, 0, 2, &[vec![3u8; 8]]);
        cut.truncate(cut.len() - 3);
        assert!(
            Ogg::parse(&cut, &Limits::default())
                .err()
                .expect("a truncated page")
                .to_string()
                .contains("states")
        );
        // A second page that starts a serial with the continuation flag set when
        // nothing is open on it.
        let lone = page(1, 0, 0, 2 | 1, &[vec![3u8; 8]]);
        assert!(
            Ogg::parse(&[lone.clone(), lone].concat(), &Limits::default())
                .err()
                .expect("a packet that never began")
                .to_string()
                .contains("never began")
        );
        // An end-of-stream flag arriving while the packet is still being written:
        // two pages of one whole 255-byte segment each, which the lacing leaves open.
        let open = [
            sealed(1, 0, 0, 2, &[255], &[3u8; 255]),
            sealed(1, 1, 0, 4 | 1, &[255], &[3u8; 255]),
        ]
        .concat();
        assert!(
            Ogg::parse(&open, &Limits::default())
                .err()
                .expect("a stream that ends unfinished")
                .to_string()
                .contains("still open")
        );
        // Anything but version 0 of the framing.
        let mut version = page(1, 0, 0, 2, &[vec![3u8; 8]]);
        version[4] = 1;
        assert!(
            Ogg::parse(&version, &Limits::default())
                .err()
                .expect("a version this reader does not walk")
                .to_string()
                .contains("version")
        );
        // And a file that is not an Ogg container at all.
        assert!(Ogg::parse(b"RIFF", &Limits::default()).is_err());
    }

    /// Where a file is cut short, the packets its pages finished are still there
    /// and the unfinished run is not a packet: no page of the serial said the stream
    /// was over, and a reader that guessed past the cut would hand out half a block.
    #[test]
    fn a_stream_cut_off_mid_packet_holds_back_the_tail() {
        let bytes = [
            sealed(1, 0, 0, 2, &[10], &[3u8; 10]),
            sealed(1, 1, -1, 1, &[255], &[7u8; 255]),
        ]
        .concat();
        let ogg = parse(&bytes);
        let chain = &ogg.chains()[0];
        assert_eq!(chain.packets.len(), 1, "the unfinished run is no packet");
        assert_eq!(chain.packets[0].data, vec![3u8; 10]);
        assert!(!chain.ended, "no page of this serial ended the stream");
    }

    #[test]
    fn the_packet_limit_stops_a_file_that_states_too_many() {
        let bytes = page(1, 0, 0, 2, &[vec![3u8; 4], vec![4u8; 4], vec![5u8; 4]]);
        let limits = Limits {
            packets: 2,
            ..Limits::default()
        };
        assert!(Ogg::parse(&bytes, &limits).is_err());
        assert_eq!(
            Ogg::parse(&bytes, &Limits::default()).unwrap().chains()[0]
                .packets
                .len(),
            3
        );
    }
}
