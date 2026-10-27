//! Owned MPEG-4 EP SRCPC channel code, ISO/IEC 14496-3 §1.8.4.6.
//!
//! Rates 8/8 through 8/32, optional four-bit tails, continuous class state and
//! joint hard-decision maximum-likelihood decoding.
use super::{Result, invalid};

const PATTERNS: [u8; 9] = [0x00, 0x80, 0x88, 0xa8, 0xaa, 0xea, 0xee, 0xfe, 0xff];
fn masks(rate: u8) -> Result<[u8; 4]> {
    if rate > 24 {
        return Err(invalid("reserved EP SRCPC code rate"));
    }
    Ok([
        0xff,
        PATTERNS[usize::from(rate.min(8))],
        PATTERNS[usize::from(rate.saturating_sub(8).min(8))],
        PATTERNS[usize::from(rate.saturating_sub(16))],
    ])
}
fn transition(state: u8, input: u8) -> (u8, [u8; 4]) {
    let m1 = state & 1;
    let m2 = (state >> 1) & 1;
    let m3 = (state >> 2) & 1;
    let m4 = (state >> 3) & 1;
    let feedback = m4 ^ m2 ^ m1;
    (
        ((state << 1) & 15) | (input ^ feedback),
        [
            input,
            m3 ^ m2 ^ m1 ^ input,
            m3 ^ m1 ^ input,
            m3 ^ m2 ^ input,
        ],
    )
}
fn steps(information_bits: usize, terminated: bool) -> Result<usize> {
    information_bits
        .checked_add(if terminated { 4 } else { 0 })
        .ok_or_else(|| invalid("EP SRCPC length overflow"))
}
/// Exact transmitted length, including punctured tail bits when terminated.
pub fn encoded_len(information_bits: usize, rate: u8, terminated: bool) -> Result<usize> {
    let masks = masks(rate)?;
    let n = steps(information_bits, terminated)?;
    let full = (n / 8)
        .checked_mul(8 + usize::from(rate))
        .ok_or_else(|| invalid("EP SRCPC length overflow"))?;
    let partial = (0..n % 8)
        .map(|t| masks.iter().filter(|&&m| m & (0x80 >> t) != 0).count())
        .sum::<usize>();
    full.checked_add(partial)
        .ok_or_else(|| invalid("EP SRCPC length overflow"))
}
fn bits(data: &[u8]) -> Result<()> {
    if data.iter().any(|&b| b > 1) {
        return Err(invalid("EP SRCPC input contains a non-bit"));
    }
    Ok(())
}
/// One SRCPC section. A non-terminated section continues the trellis and
/// puncture period into the next section. A terminated one resets both.
#[derive(Clone, Copy, Debug)]
pub struct Section {
    pub information_bits: usize,
    pub rate: u8,
    pub terminated: bool,
}
/// Encoded lengths in transmission order, preserving eight-bit puncture periods.
pub fn encoded_lengths(sections: &[Section]) -> Result<Vec<usize>> {
    let mut phase = 0;
    let mut active_rate = 0;
    let mut sizes = Vec::new();
    for section in sections {
        let requested = masks(section.rate)?;
        let n = steps(section.information_bits, section.terminated)?;
        if n == 0 {
            sizes.push(0);
            continue;
        }
        // Keep the inherited mask until its full eight-bit period completes.
        let lead = if phase == 0 { 0 } else { n.min(8 - phase) };
        let inherited = masks(active_rate)?;
        let first = (0..lead)
            .map(|t| {
                inherited
                    .iter()
                    .filter(|&&m| m & (0x80 >> (phase + t)) != 0)
                    .count()
            })
            .sum::<usize>();
        let rest = n - lead;
        let full = (rest / 8)
            .checked_mul(8 + usize::from(section.rate))
            .ok_or_else(|| invalid("EP SRCPC length overflow"))?;
        let partial = (0..rest % 8)
            .map(|t| requested.iter().filter(|&&m| m & (0x80 >> t) != 0).count())
            .sum::<usize>();
        sizes.push(
            first
                .checked_add(full)
                .and_then(|v| v.checked_add(partial))
                .ok_or_else(|| invalid("EP SRCPC length overflow"))?,
        );
        if rest != 0 {
            active_rate = section.rate;
        }
        phase = if section.terminated {
            0
        } else {
            (phase + n % 8) % 8
        };
    }
    Ok(sizes)
}
/// Input and output are unpacked bits, in transmission order, not octets.
pub fn encode(information: &[u8], rate: u8, terminated: bool) -> Result<Vec<u8>> {
    Ok(encode_chain(
        &[information],
        &[Section {
            information_bits: information.len(),
            rate,
            terminated,
        }],
    )?
    .remove(0))
}
pub fn encode_chain(messages: &[&[u8]], sections: &[Section]) -> Result<Vec<Vec<u8>>> {
    if messages.len() != sections.len() {
        return Err(invalid("EP SRCPC section count mismatch"));
    }
    let sizes = encoded_lengths(sections)?;
    let mut output = Vec::new();
    let mut state = 0;
    let mut phase = 0;
    let mut active_masks = [0; 4];
    for ((information, section), size) in messages.iter().zip(sections).zip(sizes) {
        bits(information)?;
        if information.len() != section.information_bits {
            return Err(invalid("EP SRCPC information length mismatch"));
        }
        let requested = masks(section.rate)?;
        let mut coded = Vec::new();
        coded
            .try_reserve_exact(size)
            .map_err(|_| invalid("EP SRCPC output allocation failed"))?;
        for t in 0..steps(section.information_bits, section.terminated)? {
            if phase == 0 {
                active_masks = requested;
            }
            let input = if t < information.len() {
                information[t]
            } else {
                (state & 1) ^ ((state >> 1) & 1) ^ ((state >> 3) & 1)
            };
            let (next, values) = transition(state, input);
            state = next;
            for (mask, value) in active_masks.iter().zip(values) {
                if mask & (0x80 >> phase) != 0 {
                    coded.push(value);
                }
            }
            phase = (phase + 1) % 8;
        }
        if section.terminated {
            debug_assert_eq!(state, 0);
            phase = 0;
        }
        output.push(coded);
    }
    Ok(output)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub information: Vec<u8>,
    /// Hamming distance of the selected path, not proof of correctness.
    pub distance: usize,
}
pub fn decode(
    received: &[u8],
    information_bits: usize,
    rate: u8,
    terminated: bool,
) -> Result<Decoded> {
    Ok(decode_chain(
        &[received],
        &[Section {
            information_bits,
            rate,
            terminated,
        }],
    )?
    .remove(0))
}
/// Joint hard-decision Viterbi. Later sections can disambiguate an earlier
/// non-terminated section; per-class CRC checks belong to the EP frame decoder.
pub fn decode_chain(received: &[&[u8]], sections: &[Section]) -> Result<Vec<Decoded>> {
    if received.len() != sections.len() {
        return Err(invalid("EP SRCPC section count mismatch"));
    }
    let sizes = encoded_lengths(sections)?;
    let mut total = 0usize;
    for ((coded, section), size) in received.iter().zip(sections).zip(sizes) {
        bits(coded)?;
        if coded.len() != size {
            return Err(invalid("EP SRCPC encoded class length mismatch"));
        }
        total = total
            .checked_add(steps(section.information_bits, section.terminated)?)
            .ok_or_else(|| invalid("EP SRCPC length overflow"))?;
    }
    const INF: usize = usize::MAX / 4;
    let mut metric = [INF; 16];
    metric[0] = 0;
    let mut history = Vec::<[u8; 16]>::new();
    history
        .try_reserve_exact(total)
        .map_err(|_| invalid("EP SRCPC traceback allocation failed"))?;
    let mut phase = 0;
    let mut active_masks = [0; 4];
    for (coded, section) in received.iter().zip(sections) {
        let requested = masks(section.rate)?;
        let mut at = 0;
        for _ in 0..steps(section.information_bits, section.terminated)? {
            if phase == 0 {
                active_masks = requested;
            }
            let selected: [bool; 4] =
                std::array::from_fn(|i| active_masks[i] & (0x80 >> phase) != 0);
            let mut observation = [0; 4];
            for i in 0..4 {
                if selected[i] {
                    observation[i] = coded[at];
                    at += 1;
                }
            }
            let mut next = [INF; 16];
            let mut predecessors = [0; 16];
            for prior in 0u8..16 {
                if metric[usize::from(prior)] == INF {
                    continue;
                }
                for input in 0..=1 {
                    let (state, output) = transition(prior, input);
                    let errors = (0..4)
                        .filter(|&i| selected[i] && observation[i] != output[i])
                        .count();
                    let candidate = metric[usize::from(prior)].saturating_add(errors).min(INF);
                    if candidate < next[usize::from(state)] {
                        next[usize::from(state)] = candidate;
                        predecessors[usize::from(state)] = (prior << 1) | input;
                    }
                }
            }
            metric = next;
            history.push(predecessors);
            phase = (phase + 1) % 8;
        }
        if section.terminated {
            for value in &mut metric[1..] {
                *value = INF;
            }
            phase = 0;
        }
    }
    let mut state = (0..16).min_by_key(|&s| metric[s]).unwrap();
    if metric[state] == INF {
        return Err(invalid("EP SRCPC has no legal initial path"));
    }
    let mut information = vec![0; total];
    for t in (0..total).rev() {
        let previous = history[t][state];
        information[t] = previous & 1;
        state = usize::from(previous >> 1);
    }
    if state != 0 {
        return Err(invalid("EP SRCPC has no legal initial path"));
    }
    let mut at = 0;
    let mut messages = Vec::new();
    for section in sections {
        messages.push(information[at..at + section.information_bits].to_vec());
        at += steps(section.information_bits, section.terminated)?;
    }
    let views: Vec<_> = messages.iter().map(Vec::as_slice).collect();
    let predicted = encode_chain(&views, sections)?;
    Ok(messages
        .into_iter()
        .zip(predicted)
        .zip(received)
        .map(|((information, encoded), actual)| Decoded {
            information,
            distance: encoded
                .iter()
                .zip(actual.iter())
                .filter(|(a, b)| a != b)
                .count(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    const MANIFEST: &str =
        include_str!("../../../../tests/fixtures/playback-errors/aac-ep-srcpc/manifest.json");
    fn unpack(text: &str) -> Vec<u8> {
        text.bytes().map(|b| b - b'0').collect()
    }
    fn manifest() -> serde_json::Value {
        serde_json::from_str(MANIFEST).unwrap()
    }
    #[test]
    fn continuous_sections_match_independent_polynomial_chains() {
        let m: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/playback-errors/aac-ep-class-codes/manifest.json"
        ))
        .unwrap();
        assert_eq!(m["chains"].as_array().unwrap().len(), 327);
        for row in m["chains"].as_array().unwrap() {
            let messages: Vec<_> = row["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| unpack(v.as_str().unwrap()))
                .collect();
            let encoded: Vec<_> = row["encoded"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| unpack(v.as_str().unwrap()))
                .collect();
            let sections: Vec<_> = messages
                .iter()
                .enumerate()
                .map(|(i, m)| Section {
                    information_bits: m.len(),
                    rate: row["rates"][i].as_u64().unwrap() as u8,
                    terminated: row["terminated"][i].as_bool().unwrap(),
                })
                .collect();
            let messages_view: Vec<_> = messages.iter().map(Vec::as_slice).collect();
            assert_eq!(encode_chain(&messages_view, &sections).unwrap(), encoded);
            let encoded_view: Vec<_> = encoded.iter().map(Vec::as_slice).collect();
            let decoded = decode_chain(&encoded_view, &sections).unwrap();
            assert_eq!(
                decoded
                    .iter()
                    .map(|d| d.information.clone())
                    .collect::<Vec<_>>(),
                messages
            );
            if let Some(damaged) = row.get("damaged") {
                let data: Vec<_> = damaged
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| unpack(v.as_str().unwrap()))
                    .collect();
                let views: Vec<_> = data.iter().map(Vec::as_slice).collect();
                assert_ne!(
                    decode(
                        &data[0],
                        sections[0].information_bits,
                        sections[0].rate,
                        false
                    )
                    .unwrap()
                    .information,
                    messages[0]
                );
                let restored = decode_chain(&views, &sections).unwrap();
                assert_eq!(
                    restored
                        .iter()
                        .map(|d| d.information.clone())
                        .collect::<Vec<_>>(),
                    messages
                );
                assert_eq!(restored[0].distance, 1);
                assert_eq!(restored[1].distance, 0);
            }
        }
        assert_eq!(
            encoded_lengths(&[
                Section {
                    information_bits: 7,
                    rate: 0,
                    terminated: false
                },
                Section {
                    information_bits: 1,
                    rate: 24,
                    terminated: true
                }
            ])
            .unwrap(),
            [7, 17]
        );
    }
    #[test]
    fn independent_polynomial_vectors_cover_all_rates_and_partial_periods() {
        let m = manifest();
        assert_eq!(m["vectors"].as_array().unwrap().len(), 500);
        for v in m["vectors"].as_array().unwrap() {
            let input = unpack(v["message"].as_str().unwrap());
            let expected = unpack(v["encoded"].as_str().unwrap());
            let rate = v["rate"].as_u64().unwrap() as u8;
            let terminated = v["terminated"].as_bool().unwrap();
            assert_eq!(encode(&input, rate, terminated).unwrap(), expected, "{v}");
            assert_eq!(
                encoded_len(input.len(), rate, terminated).unwrap(),
                expected.len()
            );
            assert_eq!(
                decode(&expected, input.len(), rate, terminated).unwrap(),
                Decoded {
                    information: input,
                    distance: 0
                }
            );
        }
        for v in m["tails"].as_array().unwrap() {
            let prefix = unpack(v["prefix"].as_str().unwrap());
            let code = encode(&prefix, 24, true).unwrap();
            let actual: Vec<_> = (4..8).map(|i| code[4 * i]).collect();
            assert_eq!(
                actual,
                unpack(v["tail"].as_str().unwrap()),
                "state {}",
                v["state"]
            );
        }
    }
    #[test]
    fn viterbi_corrects_every_single_bit_error_in_a_nonzero_terminated_class() {
        let input: Vec<_> = (0..64).map(|i| u8::from(i % 7 < 3)).collect();
        let encoded = encode(&input, 24, true).unwrap();
        for at in 0..encoded.len() {
            let mut broken = encoded.clone();
            broken[at] ^= 1;
            assert_eq!(
                decode(&broken, input.len(), 24, true).unwrap(),
                Decoded {
                    information: input.clone(),
                    distance: 1
                },
                "flipped transmitted bit {at}"
            );
        }
    }
    #[test]
    fn distance_is_globally_minimal_against_exhaustive_short_messages() {
        for rate in 0..=24 {
            for terminated in [false, true] {
                let messages: Vec<Vec<u8>> = (0..16)
                    .map(|m| (0..4).map(|i| (m >> i) & 1).collect())
                    .collect();
                let encoded: Vec<_> = messages
                    .iter()
                    .map(|m| encode(m, rate, terminated).unwrap())
                    .collect();
                for clean in &encoded {
                    let mut received = clean.clone();
                    let last = received.len() - 1;
                    received[last] ^= 1;
                    let best = encoded
                        .iter()
                        .map(|c| c.iter().zip(&received).filter(|(a, b)| a != b).count())
                        .min()
                        .unwrap();
                    let result = decode(&received, 4, rate, terminated).unwrap();
                    assert_eq!(result.distance, best);
                    let chosen = encode(&result.information, rate, terminated).unwrap();
                    assert_eq!(
                        chosen.iter().zip(&received).filter(|(a, b)| a != b).count(),
                        best
                    );
                }
            }
        }
        assert!(decode(&[1, 0, 1], 4, 0, false).is_err());
        assert!(decode(&[2], 1, 0, false).is_err());
        assert!(encode(&[2], 0, false).is_err());
        assert!(encode(&[0], 25, false).is_err());
        assert!(encoded_len(usize::MAX, 24, true).is_err());
        assert!(encoded_len(usize::MAX, 24, false).is_err());
    }
    fn compare_pcm(actual: &[u8]) {
        let expected =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-ep-srcpc/pcm.f32le");
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
    #[test]
    fn protected_nonzero_eld_packets_recover_pcm_through_the_owned_channel_decoder() {
        let m = manifest();
        let packets =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-ep-srcpc/packets.bin");
        let asc =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-ep-srcpc/plain.asc");
        let mut decoder = super::super::NativeAacDecoder::new(asc).unwrap();
        let mut pcm = vec![];
        for frame in m["frames"].as_array().unwrap() {
            let start = frame["offset"].as_u64().unwrap() as usize;
            let count = frame["bytes"].as_u64().unwrap() as usize;
            let mut received: Vec<_> = packets[start..start + count]
                .iter()
                .flat_map(|&byte| (0..8).rev().map(move |i| (byte >> i) & 1))
                .collect();
            received[count * 4] ^= 1;
            let recovered = decode(
                &received,
                frame["information_bits"].as_u64().unwrap() as usize,
                24,
                true,
            )
            .unwrap();
            assert_eq!(recovered.distance, 1);
            assert!(super::super::aac_ep_crc::verify(&recovered.information, 16).unwrap());
            let payload_bits = frame["payload_bits"].as_u64().unwrap() as usize;
            let raw: Vec<u8> = recovered.information[..payload_bits]
                .chunks_exact(8)
                .map(|row| row.iter().fold(0, |b, &bit| (b << 1) | bit))
                .collect();
            for sample in decoder.decode(&raw).unwrap() {
                pcm.extend_from_slice(&sample.to_le_bytes());
            }
        }
        compare_pcm(&pcm);
    }
    #[test]
    fn protected_movie_and_plain_companion_match_pcm_through_ep_dispatch() {
        use std::io::Cursor;
        let mut pcm = vec![];
        crate::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(include_bytes!(
                "../../../../tests/fixtures/playback-errors/aac-ep-srcpc/plain.mp4"
            )),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        compare_pcm(&pcm);
        pcm.clear();
        crate::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(include_bytes!(
                "../../../../tests/fixtures/playback-errors/aac-ep-srcpc/protected.mp4"
            )),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        compare_pcm(&pcm);
    }
    #[test]
    fn protected_movie_decodes_complete_pcm_with_ep_routing() {
        let mut pcm = vec![];
        crate::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(include_bytes!(
                "../../../../tests/fixtures/playback-errors/aac-ep-srcpc/protected.mp4"
            )),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        compare_pcm(&pcm);
    }
}
