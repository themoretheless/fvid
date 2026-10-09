//! ADTS raw-block and multiplexed-header error protection (ISO/IEC 13818-7 §8.1.1.1).
//! Uses the owned syntax readers; no PCM synthesis or predictor state is changed.
use super::{bits::BitReader, config::AacConfig, invalid, Result};

/// A protected bit span, zero padded after `end` to `width` bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub width: usize,
}

/// Ordered raw-block protection spans. Element IDs and ID_END are excluded.
/// CPE's second ICS intentionally overlaps the first 192-bit region.
pub fn regions(payload: &[u8], configuration: &[u8]) -> Result<Vec<Region>> {
    scan(payload, configuration, false).map(|(regions, _, _)| regions)
}

fn scan(
    payload: &[u8],
    configuration: &[u8],
    inspect_sbr: bool,
) -> Result<(Vec<Region>, usize, bool)> {
    let config = AacConfig::parse(configuration)?;
    let mut bits = BitReader::new(payload);
    let mut regions = Vec::new();
    let mut has_sbr = false;
    loop {
        let kind = bits.read(3)?;
        let start = bits.position();
        let mut right = None;
        match kind {
            0 | 3 => {
                bits.skip(4)?;
                super::aac_channel::ChannelData::read(&mut bits, &config)?;
            }
            1 => {
                bits.skip(4)?;
                right =
                    Some(super::aac_pair::ChannelPair::read_with_right_span(&mut bits, &config)?.1);
            }
            2 => {
                super::aac_coupling_syntax::Coupling::read(&mut bits, &config)?;
            }
            4 => super::aac_pce::skip_data_stream(&mut bits)?,
            5 => {
                super::aac_pce::ProgramConfig::read(&mut bits, 0)?;
            }
            6 => {
                if inspect_sbr {
                    super::aac_pce::read_fill(&mut bits, |input, end, _| {
                        has_sbr = true;
                        input.skip(end - input.position())
                    })?;
                } else {
                    // FIL is not ADTS protected; its count still bounds the next element.
                    let mut count = bits.read(4)? as usize;
                    if count == 15 {
                        count += bits.read(8)? as usize;
                        count -= 1;
                    }
                    bits.skip(count * 8)?;
                }
            }
            7 => break,
            _ => unreachable!(),
        }
        let end = bits.position();
        if kind <= 5 {
            let width = if kind <= 3 { 192 } else { end - start };
            regions.push(Region {
                start,
                end: end.min(start + width),
                width,
            });
            if let Some(right) = right {
                regions.push(Region {
                    start: right.start,
                    end: right.end.min(right.start + 128),
                    width: 128,
                });
            }
        }
    }
    Ok((regions, bits.position().div_ceil(8), has_sbr))
}

/// Byte extent through ID_END and raw-block byte alignment. Needed when an
/// unprotected ADTS transport frame multiplexes several variable-length blocks.
pub fn raw_block_bytes(payload: &[u8], configuration: &[u8]) -> Result<usize> {
    scan(payload, configuration, false).map(|(_, bytes, _)| bytes)
}

/// Locate a bounded SBR FIL without synthesizing PCM. The caller must validate
/// the candidate SBR data before publishing a negotiated output clock.
pub fn has_sbr_fill(payload: &[u8], configuration: &[u8]) -> Result<bool> {
    scan(payload, configuration, true).map(|(_, _, sbr)| sbr)
}

fn bit(crc: u16, value: bool) -> u16 {
    let feedback = (crc & 0x8000 != 0) ^ value;
    (crc << 1) ^ if feedback { 0x8005 } else { 0 }
}

/// Compute the transmitted CRC value, initialized to all ones, MSB first,
/// with no final inversion. Header includes all seven fixed/variable bytes.
pub fn checksum(header: &[u8; 7], payload: &[u8], configuration: &[u8]) -> Result<u16> {
    let frame = super::adts::header(header).ok_or_else(|| invalid("invalid ADTS CRC header"))?;
    if frame.header_bytes != 9 || payload.len() != frame.frame_bytes - 9 {
        return Err(invalid(
            "ADTS CRC frame length or protection flag disagrees",
        ));
    }
    let spans = regions(payload, configuration)?;
    let mut crc = 0xffff;
    for byte in header {
        for shift in (0..8).rev() {
            crc = bit(crc, byte & (1 << shift) != 0);
        }
    }
    Ok(feed_regions(crc, payload, spans))
}

fn feed_regions(mut crc: u16, payload: &[u8], spans: Vec<Region>) -> u16 {
    for span in spans {
        for position in span.start..span.end {
            crc = bit(crc, payload[position / 8] & (1 << (7 - position % 8)) != 0);
        }
        for _ in (span.end - span.start)..span.width {
            crc = bit(crc, false);
        }
    }
    crc
}

/// Per-block CRC for multiplexed ADTS: protected raw regions without headers.
pub fn raw_block_checksum(payload: &[u8], configuration: &[u8]) -> Result<u16> {
    Ok(feed_regions(
        0xffff,
        payload,
        regions(payload, configuration)?,
    ))
}

/// Multiplexed ADTS header CRC includes fixed/variable headers and positions.
pub fn header_checksum(header: &[u8; 7], positions: &[u8]) -> u16 {
    let mut crc = 0xffff;
    for byte in header.iter().chain(positions) {
        for shift in (0..8).rev() {
            crc = bit(crc, byte & (1 << shift) != 0);
        }
    }
    crc
}

/// Reject corrupted single-block ADTS before handing its payload to a decoder.
pub fn verify(header: &[u8; 7], stored: u16, payload: &[u8], configuration: &[u8]) -> Result<()> {
    if checksum(header, payload, configuration)? != stored {
        return Err(invalid("ADTS CRC mismatch"));
    }
    Ok(())
}

/// Validate a candidate implicit SBR payload and return its output rate.
/// Non-LC and explicitly signalled configurations do not require discovery.
pub fn probe_sbr_rate(payload: &[u8], configuration: &[u8]) -> Result<Option<u32>> {
    let config = super::config::AudioSpecificConfig::parse(configuration)?;
    if config.core.object_type != 2 || config.sbr_present.is_some() {
        return Ok(None);
    }
    if !has_sbr_fill(payload, configuration)? {
        return Ok(None);
    }
    let mut decoder = super::NativeAacDecoder::new_with_sbr_detection(configuration)?;
    decoder.decode(payload)?;
    Ok(Some(decoder.sample_rate()))
}
