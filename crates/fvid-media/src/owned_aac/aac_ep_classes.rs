//! EP2 classes reassemble the epConfig=0 stream (14496-3 section 1.8.1).
use super::{Class, Config, Length, Result, decode_frame_header, decode_rs_class, invalid};
use crate::owned_aac::{aac_ep_crc, aac_ep_interleave as stage, aac_ep_rs, aac_ep_srcpc as srcpc};

pub(super) fn validate(config: &Config) -> Result<()> {
    if config.sets.is_empty() || config.sets.len() > 255 {
        return Err(invalid("EP predefined set count out of range"));
    }
    if config.interleave > 2 {
        return Err(invalid("reserved EP interleave mode"));
    }
    if config.frames != 1 {
        return Err(invalid(
            "EP concatenated frame routing is not yet implemented",
        ));
    }
    for set in &config.sets {
        let count = set.classes.len();
        if count > 63 {
            return Err(invalid("EP class count out of range"));
        }
        if let Some(order) = &set.order {
            if order.len() != count {
                return Err(invalid("invalid EP class output permutation"));
            }
            let mut seen = 0u64;
            for &k in order {
                if usize::from(k) >= count || seen & (1u64 << k) != 0 {
                    return Err(invalid("invalid EP class output permutation"));
                }
                seen |= 1u64 << k;
            }
        }
        for (index, c) in set.classes.iter().enumerate() {
            if c.optional {
                return Err(invalid(
                    "EP optional class derivation is not yet implemented",
                ));
            }
            if c.fec > 2 {
                return Err(invalid("reserved EP FEC type"));
            }
            if c.fec == 2 {
                return Err(invalid(
                    "EP RS concatenated class routing is not yet implemented",
                ));
            }
            match (config.interleave, c.interleave) {
                (0 | 1, None) | (2, Some(0..=3)) => (),
                _ => return Err(invalid("invalid EP class interleave switch")),
            }
            if c.fec != 0 && c.interleave == Some(2) {
                return Err(invalid("reserved RS intraclass interleave mode"));
            }
            if c.fec == 1 && c.rate.is_none() {
                return Err(invalid(
                    "EP RS escaped correction rate is not yet implemented",
                ));
            }
            if c.fec == 0 && c.rate.is_some_and(|r| r > 24) {
                return Err(invalid("reserved EP SRCPC code rate"));
            }
            if c.crc.is_some_and(|w| w > 16 && w != 24 && w != 32) {
                return Err(invalid("reserved EP CRC width"));
            }
            if matches!(c.length, Length::InBand(16..)) {
                return Err(invalid("EP length field width out of range"));
            }
            let last = set
                .order
                .as_ref()
                .and_then(|o| o.last())
                .map(|&v| usize::from(v))
                .unwrap_or(count.saturating_sub(1));
            if c.length == Length::InBand(0) && index != last {
                return Err(invalid(
                    "EP until-end class is not last in transmission order",
                ));
            }
        }
    }
    Ok(())
}
struct Parameter<'a> {
    class: &'a Class,
    output: usize,
    length: Option<usize>,
    rate: u8,
    crc: u8,
}
fn field(header: &[u8], at: &mut usize, n: usize) -> usize {
    let value = header[*at..*at + n]
        .iter()
        .fold(0, |v, &b| (v << 1) | usize::from(b));
    *at += n;
    value
}
fn size(p: &Parameter<'_>, prefix: &[srcpc::Section], n: usize) -> Result<usize> {
    // A zero-bit EP class has no CRC, tail, parity or trellis advance.
    if n == 0 {
        return Ok(0);
    }
    let information = n
        .checked_add(usize::from(p.crc))
        .ok_or_else(|| invalid("EP class length overflow"))?;
    if p.class.fec == 1 {
        if n % 8 != 0 || p.crc % 8 != 0 {
            return Err(invalid(
                "EP RS raw-AU mapping requires octet-aligned information and CRC",
            ));
        }
        return aac_ep_rs::encoded_len(information / 8, p.rate)?
            .checked_mul(8)
            .ok_or_else(|| invalid("EP class length overflow"));
    }
    let mut sections = prefix.to_vec();
    sections.push(srcpc::Section {
        information_bits: information,
        rate: p.rate,
        terminated: p.class.terminated,
    });
    Ok(*srcpc::encoded_lengths(&sections)?.last().unwrap())
}
fn length_error(parameters: &[Parameter<'_>]) -> crate::owned_aac::Error {
    invalid(if parameters.last().is_some_and(|p| p.class.fec == 1) {
        "EP RS class/packet length mismatch"
    } else {
        "EP class/packet length mismatch"
    })
}
fn lengths(parameters: &mut [Parameter<'_>], total: usize) -> Result<Vec<usize>> {
    let mut prefix = Vec::new();
    let mut sizes = Vec::new();
    let mut used = 0usize;
    for i in 0..parameters.len() {
        let p = &parameters[i];
        let n = if let Some(n) = p.length {
            n
        } else {
            let remaining = total
                .checked_sub(used)
                .ok_or_else(|| length_error(parameters))?;
            let unit = if p.class.fec == 1 { 8 } else { 1 };
            let mut low = 0;
            let mut high = remaining / unit;
            while low < high {
                let middle = low + (high - low) / 2;
                if size(p, &prefix, middle * unit)? >= remaining {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            if size(p, &prefix, low * unit)? != remaining {
                return Err(length_error(parameters));
            }
            low * unit
        };
        let encoded = size(p, &prefix, n)?;
        used = used
            .checked_add(encoded)
            .ok_or_else(|| invalid("EP class length overflow"))?;
        if p.class.fec == 0 && n != 0 {
            prefix.push(srcpc::Section {
                information_bits: n + usize::from(p.crc),
                rate: p.rate,
                terminated: p.class.terminated,
            });
        }
        parameters[i].length = Some(n);
        sizes.push(encoded);
    }
    if used != total {
        return Err(length_error(parameters));
    }
    Ok(sizes)
}
fn undo(bits: &[u8], p: &Parameter<'_>, count: usize, mode: u8) -> Result<(Vec<u8>, Vec<u8>)> {
    if count == 0 {
        return Ok((vec![], bits.to_vec()));
    }
    if p.class.fec == 1 {
        stage::split_octet_stage(bits, count)
    } else {
        let width = if mode == 1 || p.class.interleave == Some(2) {
            28
        } else {
            count
        };
        stage::split_stage(bits, count, width)
    }
}
fn split_body(
    mut body: Vec<u8>,
    parameters: &[Parameter<'_>],
    sizes: &[usize],
    mode: u8,
) -> Result<Vec<Vec<u8>>> {
    let mut coded = vec![vec![]; parameters.len()];
    if mode == 0 {
        let mut at = 0;
        for (i, &n) in sizes.iter().enumerate() {
            coded[i] = body[at..at + n].to_vec();
            at += n;
        }
        return Ok(coded);
    }
    if mode == 1 {
        for (i, p) in parameters.iter().enumerate() {
            let (x, y) = undo(&body, p, sizes[i], mode)?;
            coded[i] = x;
            body = y;
        }
        if !body.is_empty() {
            return Err(invalid("unconsumed EP interleave data"));
        }
        return Ok(coded);
    }
    let no_len = parameters
        .iter()
        .zip(sizes)
        .filter(|(p, _)| p.class.interleave == Some(0))
        .map(|(_, n)| *n)
        .sum::<usize>();
    let no = body.split_off(body.len() - no_len);
    let mut at = 0;
    for (i, p) in parameters.iter().enumerate() {
        if p.class.interleave == Some(0) {
            coded[i] = no[at..at + sizes[i]].to_vec();
            at += sizes[i];
        }
    }
    for (i, p) in parameters.iter().enumerate() {
        if matches!(p.class.interleave, Some(1 | 2)) {
            let (x, y) = undo(&body, p, sizes[i], mode)?;
            coded[i] = x;
            body = y;
        }
    }
    let mut at = 0;
    for (i, p) in parameters.iter().enumerate() {
        if p.class.interleave == Some(3) {
            coded[i] = body[at..at + sizes[i]].to_vec();
            at += sizes[i];
        }
    }
    if at != body.len() {
        return Err(invalid("unconsumed EP concatenated data"));
    }
    Ok(coded)
}
pub(super) fn decode(config: &Config, packet: &[u8]) -> Result<Vec<u8>> {
    validate(config)?;
    let bits = packet
        .iter()
        .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect();
    let n = (usize::BITS - (config.sets.len() - 1).leading_zeros()) as usize;
    let (selector, following) = decode_frame_header(bits, n, None, config.interleave != 0)?;
    let selected = selector
        .iter()
        .fold(0usize, |v, &b| (v << 1) | usize::from(b));
    let set = config
        .sets
        .get(selected)
        .ok_or_else(|| invalid("EP predefined set index out of range"))?;
    let order: Vec<_> = set
        .order
        .as_ref()
        .map(|o| o.iter().map(|&v| usize::from(v)).collect())
        .unwrap_or_else(|| (0..set.classes.len()).collect());
    let n=set.classes.iter().map(|c|match c.length{Length::InBand(n)=>usize::from(n),_=>0}+3*usize::from(c.rate.is_none())+3*usize::from(c.crc.is_none())).sum::<usize>()+3*usize::from(config.stuffing);
    let (header, mut body) =
        decode_frame_header(following, n, config.header, config.interleave != 0)?;
    let mut parameters = Vec::new();
    let mut at = 0;
    for index in order {
        let class = &set.classes[index];
        let length = match class.length {
            Length::Fixed(n) => Some(usize::from(n)),
            Length::InBand(0) => None,
            Length::InBand(n) => Some(field(&header, &mut at, usize::from(n))),
        };
        let rate = class
            .rate
            .unwrap_or_else(|| [0, 3, 4, 6, 8, 12, 16, 24][field(&header, &mut at, 3)]);
        let crc = class
            .crc
            .unwrap_or_else(|| [0, 6, 8, 10, 12, 14, 16, 32][field(&header, &mut at, 3)]);
        parameters.push(Parameter {
            class,
            output: index,
            length,
            rate,
            crc,
        });
    }
    let stuffing = if config.stuffing {
        field(&header, &mut at, 3)
    } else {
        0
    };
    body.truncate(
        body.len()
            .checked_sub(stuffing)
            .ok_or_else(|| invalid("truncated EP stuffing/header"))?,
    );
    let sizes = lengths(&mut parameters, body.len())?;
    let coded = split_body(body, &parameters, &sizes, config.interleave)?;
    let srcpc_indices: Vec<_> = parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| p.class.fec == 0 && p.length != Some(0))
        .map(|(i, _)| i)
        .collect();
    let sections: Vec<_> = srcpc_indices
        .iter()
        .map(|&i| {
            let p = &parameters[i];
            srcpc::Section {
                information_bits: p.length.unwrap() + usize::from(p.crc),
                rate: p.rate,
                terminated: p.class.terminated,
            }
        })
        .collect();
    let views: Vec<_> = srcpc_indices.iter().map(|&i| coded[i].as_slice()).collect();
    let recovered = srcpc::decode_chain(&views, &sections)?;
    let mut output = vec![vec![]; set.classes.len()];
    for (&i, recovered) in srcpc_indices.iter().zip(recovered) {
        let p = &parameters[i];
        if !aac_ep_crc::verify(&recovered.information, p.crc)? {
            return Err(invalid("EP class CRC mismatch"));
        }
        output[p.output] = recovered.information[..p.length.unwrap()].to_vec();
    }
    for (i, p) in parameters.iter().enumerate() {
        if p.class.fec == 1 && p.length != Some(0) {
            let raw = decode_rs_class(&coded[i], p.length, p.rate, p.crc)?;
            output[p.output] = raw
                .iter()
                .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
                .collect();
        }
    }
    let total = output
        .iter()
        .try_fold(0usize, |sum, row| sum.checked_add(row.len()))
        .ok_or_else(|| invalid("EP raw-AU length overflow"))?;
    if total % 8 != 0 {
        return Err(invalid("EP raw access unit is not octet aligned"));
    }
    let mut raw = vec![0; total / 8];
    let mut at = 0;
    for row in output {
        for b in row {
            raw[at / 8] |= b << (7 - at % 8);
            at += 1;
        }
    }
    Ok(raw)
}
