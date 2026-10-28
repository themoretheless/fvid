//! Exact EP class recovery and EP2 raw-AU assembly (14496-3 section 1.8.1).
use super::{
    Class, Config, Length, RecoveredClass, RecoveredFrame, Result, decode_frame_header,
    decode_rs_class_bits, invalid,
};
use crate::owned_aac::{aac_ep_crc, aac_ep_interleave as stage, aac_ep_rs, aac_ep_srcpc as srcpc};

pub(super) fn validate(config: &Config) -> Result<()> {
    if config.sets.is_empty() || config.sets.len() > 255 {
        return Err(invalid("EP predefined set count out of range"));
    }
    if config.interleave > 2 {
        return Err(invalid("reserved EP interleave mode"));
    }
    if !(1..=7).contains(&config.frames) {
        return Err(invalid("reserved EP concatenated frame count"));
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
            if config.frames > 1
                && (!matches!(c.length, Length::Fixed(_)) || c.rate.is_none() || c.crc.is_none())
            {
                return Err(invalid(
                    "EP concatenated frames forbid escaped class parameters",
                ));
            }
            if c.fec > 2 {
                return Err(invalid("reserved EP FEC type"));
            }
            match (config.interleave, c.interleave) {
                (0 | 1, None) | (2, Some(0..=3)) => (),
                _ => return Err(invalid("invalid EP class interleave switch")),
            }
            if c.fec != 0 && c.interleave == Some(2) {
                return Err(invalid("reserved RS intraclass interleave mode"));
            }
            if c.fec != 0 && c.rate.is_none() {
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
/// Wire cardinality after optional-class derivation. The stored descriptors
/// stay original; even 255 sets of 63 optional classes require only a count.
pub(super) fn predefined_count(config: &Config) -> Result<u128> {
    if config.sets.is_empty() || config.sets.len() > 255 {
        return Err(invalid("EP predefined set count out of range"));
    }
    let mut total = 0u128;
    for set in &config.sets {
        if set.classes.len() > 63 {
            return Err(invalid("EP class count out of range"));
        }
        let optional = set.classes.iter().filter(|c| c.optional).count();
        total = total
            .checked_add(1u128 << optional)
            .ok_or_else(|| invalid("EP derived set count overflow"))?;
    }
    Ok(total)
}
fn select_derived(config: &Config, mut selected: u128) -> Result<(usize, Vec<usize>)> {
    if selected >= predefined_count(config)? {
        return Err(invalid("EP predefined set index out of range"));
    }
    for (original, set) in config.sets.iter().enumerate() {
        let count = 1u128 << set.classes.iter().filter(|c| c.optional).count();
        if selected >= count {
            selected -= count;
            continue;
        }
        let mut rank = 0;
        let mut present = Vec::new();
        for (index, class) in set.classes.iter().enumerate() {
            let keep = if class.optional {
                let keep = selected & (1u128 << rank) == 0;
                rank += 1;
                keep
            } else {
                true
            };
            if keep {
                present.push(index);
            }
        }
        return Ok((original, present));
    }
    Err(invalid("EP predefined set index out of range"))
}

struct Parameter<'a> {
    class: &'a Class,
    output: usize,
    length: Option<usize>,
    rate: u8,
    crc: u8,
    members: Vec<Parameter<'a>>,
}
// ISO 1.8.4.7: successive FEC=2 classes and the terminating FEC=1
// class are one RS information stream, including each member's own CRC.
// Group before computing parity geometry or applying the recursive interleaver.
fn rs_groups(parameters: Vec<Parameter<'_>>) -> Result<Vec<Parameter<'_>>> {
    let mut result = Vec::new();
    let mut input = parameters.into_iter();
    while let Some(first) = input.next() {
        if first.class.fec != 2 {
            result.push(first);
            continue;
        }
        let mut group = Parameter {
            class: first.class,
            output: first.output,
            length: Some(0),
            rate: first.rate,
            crc: 0,
            members: vec![first],
        };
        loop {
            let member = input
                .next()
                .ok_or_else(|| invalid("unterminated EP RS class group"))?;
            if member.class.fec == 0 {
                return Err(invalid("EP RS class group ends in SRCPC class"));
            }
            if member.rate != group.rate {
                return Err(invalid("EP RS class group correction rates differ"));
            }
            if member.class.interleave != group.class.interleave {
                return Err(invalid("EP RS class group interleave switches differ"));
            }
            let last = member.class.fec == 1;
            group.members.push(member);
            if last {
                break;
            }
        }
        let mut total = Some(0usize);
        for member in &group.members {
            total = match (total, member.length) {
                (Some(sum), Some(n)) => Some(
                    sum.checked_add(n + if n == 0 { 0 } else { usize::from(member.crc) })
                        .ok_or_else(|| invalid("EP RS class length overflow"))?,
                ),
                _ => None,
            };
        }
        // CRC is appended before FEC. Only the complete RS information word
        // is octet based; member and CRC boundaries can be fractional octets.
        if total.is_some_and(|n| n % 8 != 0) {
            return Err(invalid("EP RS information plus CRC is not octet aligned"));
        }
        group.length = total;
        result.push(group);
    }
    Ok(result)
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
    if p.class.fec != 0 {
        if information % 8 != 0 {
            return Err(invalid("EP RS information plus CRC is not octet aligned"));
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
    invalid(if parameters.last().is_some_and(|p| p.class.fec != 0) {
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
            let unit = if p.class.fec != 0 { 8 } else { 1 };
            // An until-end RS class includes its CRC in the octet count.
            // Search data lengths congruent to -CRC, not only byte lengths.
            let offset = if p.class.fec != 0 {
                (8 - usize::from(p.crc) % 8) % 8
            } else {
                0
            };
            if remaining == 0 {
                0
            } else {
                let mut low = 0;
                let mut high = remaining / unit;
                while low < high {
                    let middle = low + (high - low) / 2;
                    if size(p, &prefix, middle * unit + offset)? >= remaining {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                if size(p, &prefix, low * unit + offset)? != remaining {
                    return Err(length_error(parameters));
                }
                low * unit + offset
            }
        };
        let minimum = p
            .members
            .iter()
            .try_fold(0usize, |sum, m| {
                let known = m.length.unwrap_or(0);
                sum.checked_add(known + if known == 0 { 0 } else { usize::from(m.crc) })
            })
            .ok_or_else(|| invalid("EP RS class length overflow"))?;
        if n < minimum {
            return Err(invalid("EP RS class/packet length mismatch"));
        }
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
    if p.class.fec != 0 {
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
    if config.frames != 1 {
        validate(config)?;
        return Err(invalid(
            "EP frame contains multiple raw access units; use decode_raw_access_units",
        ));
    }
    Ok(decode_units(config, packet)?.remove(0))
}
pub(super) fn decode_units(config: &Config, packet: &[u8]) -> Result<Vec<Vec<u8>>> {
    decode_class_frames(config, packet)?
        .into_iter()
        .map(|frame| {
            let total = frame
                .classes
                .iter()
                .try_fold(0usize, |sum, row| sum.checked_add(row.bits.len()))
                .ok_or_else(|| invalid("EP raw-AU length overflow"))?;
            if total % 8 != 0 {
                return Err(invalid("EP raw access unit is not octet aligned"));
            }
            let mut raw = vec![0; total / 8];
            for (at, bit) in frame
                .classes
                .into_iter()
                .flat_map(|row| row.bits)
                .enumerate()
            {
                raw[at / 8] |= bit << (7 - at % 8);
            }
            Ok(raw)
        })
        .collect()
}
pub(super) fn decode_class_frames(config: &Config, packet: &[u8]) -> Result<Vec<RecoveredFrame>> {
    validate(config)?;
    let bits = packet
        .iter()
        .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect();
    let count = predefined_count(config)?;
    let n = (u128::BITS - (count - 1).leading_zeros()) as usize;
    let (selector, following) =
        decode_frame_header(bits, n, config.header, config.interleave != 0)?;
    let selected = selector
        .iter()
        .fold(0u128, |v, &b| (v << 1) | u128::from(b));
    let (original, present) = select_derived(config, selected)?;
    let set = &config.sets[original];
    let mask = present
        .iter()
        .fold(0u64, |mask, &index| mask | (1u64 << index));
    let order: Vec<_> = set
        .order
        .as_ref()
        .map(|o| {
            o.iter()
                .map(|&v| usize::from(v))
                .filter(|&index| mask & (1u64 << index) != 0)
                .collect()
        })
        .unwrap_or(present);
    let n = order
        .iter()
        .map(|&index| {
            let c = &set.classes[index];
            (match c.length {
                Length::InBand(n) => usize::from(n),
                _ => 0,
            }) + 3 * usize::from(c.rate.is_none())
                + 3 * usize::from(c.crc.is_none())
        })
        .sum::<usize>()
        + 3 * usize::from(config.stuffing);
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
        let frames = usize::from(config.frames);
        let joined = frames > 1 && class.concatenated;
        for frame in 0..if joined { 1 } else { frames } {
            parameters.push(Parameter {
                class,
                output: index * frames + frame,
                length: if joined {
                    length.map(|n| n * frames)
                } else {
                    length
                },
                rate,
                crc,
                members: Vec::new(),
            });
        }
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
    let mut parameters = rs_groups(parameters)?;
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
    let frames = usize::from(config.frames);
    let mut output = vec![vec![]; set.classes.len() * frames];
    for (&i, recovered) in srcpc_indices.iter().zip(recovered) {
        let p = &parameters[i];
        if !aac_ep_crc::verify(&recovered.information, p.crc)? {
            return Err(invalid("EP class CRC mismatch"));
        }
        output[p.output] = recovered.information[..p.length.unwrap()].to_vec();
    }
    for (i, p) in parameters.iter().enumerate() {
        if p.class.fec != 0 && p.length != Some(0) {
            let bits = decode_rs_class_bits(&coded[i], p.length, p.rate, p.crc)?;
            if p.members.is_empty() {
                output[p.output] = bits;
            } else {
                let mut at = 0usize;
                for member in &p.members {
                    let n = member
                        .length
                        .unwrap_or_else(|| bits.len().saturating_sub(at + usize::from(member.crc)));
                    let width = if n == 0 { 0 } else { usize::from(member.crc) };
                    let end = at
                        .checked_add(n)
                        .and_then(|n| n.checked_add(width))
                        .filter(|&n| n <= bits.len())
                        .ok_or_else(|| invalid("EP RS class/packet length mismatch"))?;
                    if !aac_ep_crc::verify(&bits[at..end], width as u8)? {
                        return Err(invalid("EP class CRC mismatch"));
                    }
                    output[member.output] = bits[at..at + n].to_vec();
                    at = end;
                }
                if at != bits.len() {
                    return Err(invalid("EP RS class/packet length mismatch"));
                }
            }
        }
    }
    let mut units = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut classes = Vec::with_capacity(set.classes.len());
        for (index, class) in set.classes.iter().enumerate() {
            if mask & (1u64 << index) == 0 {
                continue;
            }
            let bits = if frames > 1 && class.concatenated {
                let row = &output[index * frames];
                let Length::Fixed(n) = class.length else {
                    unreachable!()
                };
                let n = usize::from(n);
                row[frame * n..(frame + 1) * n].to_vec()
            } else {
                // Each independent class belongs to exactly one source frame.
                std::mem::take(&mut output[index * frames + frame])
            };
            classes.push(RecoveredClass {
                class_index: index,
                bits,
            });
        }
        units.push(RecoveredFrame {
            set_index: original,
            classes,
        });
    }
    Ok(units)
}

#[cfg(test)]
mod optional_tests {
    use super::*;
    use crate::owned_aac::aac_ep::Set;
    fn class(optional: bool) -> Class {
        Class {
            length: Length::Fixed(8),
            rate: Some(0),
            crc: Some(0),
            concatenated: false,
            fec: 0,
            terminated: false,
            interleave: None,
            optional,
        }
    }
    fn config(sets: Vec<Set>) -> Config {
        Config {
            sets,
            interleave: 0,
            stuffing: true,
            frames: 1,
            header: None,
        }
    }
    #[test]
    fn rs_group_geometry_and_malformed_boundaries() {
        fn parameters(classes: &[Class]) -> Vec<Parameter<'_>> {
            classes
                .iter()
                .enumerate()
                .map(|(output, class)| Parameter {
                    class,
                    output,
                    length: match class.length {
                        Length::Fixed(n) => Some(usize::from(n)),
                        _ => None,
                    },
                    rate: class.rate.unwrap(),
                    crc: class.crc.unwrap(),
                    members: vec![],
                })
                .collect()
        }
        let mut a = class(false);
        a.fec = 2;
        a.rate = Some(4);
        a.crc = Some(8);
        let mut b = a.clone();
        b.fec = 1;
        b.crc = Some(16);
        let source = vec![a.clone(), b.clone(), class(false), a.clone(), b.clone()];
        let grouped = rs_groups(parameters(&source)).unwrap();
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].length, Some(40));
        assert_eq!(grouped[0].members.len(), 2);
        assert_eq!(grouped[1].class.fec, 0);
        assert_eq!(
            grouped[2]
                .members
                .iter()
                .map(|m| m.output)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(size(&grouped[0], &[], 40).unwrap(), (5 + 8) * 8);
        let cases = [
            (vec![a.clone()], "unterminated EP RS class group"),
            (
                vec![a.clone(), class(false)],
                "EP RS class group ends in SRCPC class",
            ),
            (
                {
                    let mut c = b.clone();
                    c.rate = Some(3);
                    vec![a.clone(), c]
                },
                "EP RS class group correction rates differ",
            ),
            (
                {
                    let mut c = b.clone();
                    c.interleave = Some(1);
                    vec![a.clone(), c]
                },
                "EP RS class group interleave switches differ",
            ),
            (
                {
                    let mut c = b.clone();
                    c.crc = Some(6);
                    vec![a.clone(), c]
                },
                "information plus CRC is not octet aligned",
            ),
        ];
        for (classes, expected) in cases {
            let error = rs_groups(parameters(&classes)).err().unwrap().to_string();
            assert!(error.contains(expected), "{error}");
        }
        let mut empty = vec![a, b];
        for c in &mut empty {
            c.length = Length::Fixed(0);
        }
        let grouped = rs_groups(parameters(&empty)).unwrap();
        assert_eq!(grouped[0].length, Some(0));
        assert_eq!(size(&grouped[0], &[], 0).unwrap(), 0);
    }
    fn oracle_header(information: &str) -> Vec<u8> {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-sets/manifest.json"
        ))
        .unwrap();
        m["header_vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["information"] == information)
            .unwrap()["encoded"]
            .as_str()
            .unwrap()
            .bytes()
            .map(|v| v - b'0')
            .collect()
    }
    fn pack(bits: &[u8]) -> Vec<u8> {
        bits.chunks_exact(8)
            .map(|row| row.iter().fold(0, |v, &b| (v << 1) | b))
            .collect()
    }
    #[test]
    fn normative_optional_table_and_filtered_output_order_recover_exact_labels() {
        let cfg = config(vec![
            Set {
                classes: vec![
                    class(true),
                    class(false),
                    class(true),
                    class(false),
                    class(true),
                ],
                order: Some(vec![4, 0, 1, 3, 2]),
            },
            Set {
                classes: vec![class(true), class(false)],
                order: Some(vec![1, 0]),
            },
        ]);
        assert_eq!(cfg.predefined_set_count().unwrap(), 10);
        // Table 1.58. These are EP transport labels, not AAC playback fixtures.
        for (selected, expected) in [
            "ABCDE", "BCDE", "ABDE", "BDE", "ABCD", "BCD", "ABD", "BD", "FG", "G",
        ]
        .iter()
        .enumerate()
        {
            let (original, present) = select_derived(&cfg, selected as u128).unwrap();
            let start = if original == 0 { b'A' } else { b'F' };
            assert_eq!(
                present.iter().map(|&i| start + i as u8).collect::<Vec<_>>(),
                expected.as_bytes()
            );
            let mut bits = oracle_header(&format!("{selected:04b}"));
            bits.extend(oracle_header("011"));
            for &i in cfg.sets[original].order.as_ref().unwrap() {
                if present.contains(&usize::from(i)) {
                    bits.extend((0..8).rev().map(|b| ((start + i) >> b) & 1));
                }
            }
            bits.extend([0, 0, 0]);
            assert_eq!(
                cfg.decode_raw_access_unit(&pack(&bits)).unwrap(),
                expected.as_bytes()
            );
        }
        assert!(select_derived(&cfg, 10).is_err());
        let empty = config(vec![Set {
            classes: vec![class(true)],
            order: None,
        }]);
        let mut bits = vec![1, 1, 1];
        bits.extend(oracle_header("111"));
        bits.extend([0; 7]);
        assert!(
            empty
                .decode_raw_access_unit(&pack(&bits))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn optional_cardinality_and_sparse_selection_cover_all_63_flags_without_expansion() {
        for optional in 0..=63 {
            let cfg = config(vec![Set {
                classes: (0..63).map(|i| class(i < optional)).collect(),
                order: None,
            }]);
            let count = 1u128 << optional;
            assert_eq!(cfg.predefined_set_count().unwrap(), count);
            assert_eq!(
                select_derived(&cfg, 0).unwrap().1,
                (0..63).collect::<Vec<_>>()
            );
            assert_eq!(
                select_derived(&cfg, count - 1).unwrap().1,
                (optional..63).collect::<Vec<_>>()
            );
            if optional > 0 {
                assert_eq!(
                    select_derived(&cfg, 1u128 << (optional - 1)).unwrap().1,
                    (0..63).filter(|&i| i != optional - 1).collect::<Vec<_>>()
                );
            }
        }
        let set = Set {
            classes: vec![class(true); 63],
            order: None,
        };
        let cfg = config(vec![set; 255]);
        let count = 255u128 * (1u128 << 63);
        assert_eq!(cfg.predefined_set_count().unwrap(), count);
        assert_eq!(128 - (count - 1).leading_zeros(), 71);
        assert_eq!(select_derived(&cfg, count - 1).unwrap(), (254, vec![]));
        assert_eq!(select_derived(&cfg, count - 2).unwrap(), (254, vec![0]));
        assert_eq!(
            select_derived(&cfg, 1u128 << 64).unwrap(),
            (2, (0..63).collect())
        );
        assert!(select_derived(&cfg, count).is_err());
        let mut invalid = cfg;
        invalid.sets[0].classes.push(class(true));
        assert!(invalid.predefined_set_count().is_err());
    }
}
