//! Optional owned fixture generation; ordinary tests consume saved streams.
use fvid::codec::{av1::Obus, av1_frame::Header, av1_sequence::Sequence};
fn bits(data: &[u8]) -> Vec<u8> {
    data.iter()
        .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect()
}
fn bytes(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8)
        .map(|v| v.iter().fold(0, |a, b| (a << 1) | b))
        .collect()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 6);
    let data = std::fs::read(&args[1]).unwrap();
    let levels: [u8; 3] = std::array::from_fn(|i| args[i + 3].parse().unwrap());
    assert!(levels.iter().all(|&v| v < 16));
    let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
    let seq = obus.iter().find(|o| o.kind == 1).unwrap();
    let original = Sequence::parse(seq.payload).unwrap();
    let mut intended_sequence = original.clone();
    intended_sequence.color.separate_uv_delta_q = true;
    let mut sequences = Vec::new();
    for bit in 0..seq.payload.len() * 8 {
        let mut p = seq.payload.to_vec();
        p[bit / 8] ^= 1 << (7 - bit % 8);
        if Sequence::parse(&p).is_ok_and(|s| format!("{s:?}") == format!("{intended_sequence:?}")) {
            sequences.push(p);
        }
    }
    assert_eq!(sequences.len(), 1, "unique sequence UV flag rewrite");
    let sequence_payload = sequences.pop().unwrap();
    let frame = obus.iter().find(|o| o.kind == 6).unwrap();
    let mut intended = Header::parse_intra(
        &original,
        frame.payload,
        frame.temporal_id,
        frame.spatial_id,
    )
    .unwrap();
    let had_matrix = intended.quant.matrix.is_some();
    assert!(intended.quant.matrix.is_none() || intended.quant.matrix == Some([7; 3]));
    assert_eq!(intended.quant.delta[1..3], intended.quant.delta[3..5]);
    let uv_bits = 1 + intended.quant.delta[1..3]
        .iter()
        .map(|&v| if v == 0 { 1 } else { 8 })
        .sum::<usize>();
    let old_matrix_bits = if had_matrix { 8 } else { 1 };
    let prefix_bits = if had_matrix { uv_bits } else { uv_bits - 1 };
    let old_header_bytes = intended.header_bytes;
    intended.quant.matrix = Some(levels);
    let original_bits = bits(&frame.payload[..old_header_bytes]);
    let mut matches = Vec::new();
    for bit in prefix_bits..original_bits.len() - old_matrix_bits {
        if (had_matrix && original_bits[bit..bit + 8] != [0, 1, 1, 1, 0, 1, 1, 1])
            || (!had_matrix && original_bits[bit] != 0)
        {
            continue;
        }
        for padding in 0..8 {
            if original_bits.len() - padding < bit + old_matrix_bits
                || original_bits[original_bits.len() - padding..]
                    .iter()
                    .any(|&v| v != 0)
            {
                continue;
            }
            let mut b = original_bits[..bit - prefix_bits].to_vec();
            b.push(0);
            b.extend_from_slice(&original_bits[bit - prefix_bits..bit]);
            if !had_matrix {
                b.push(1);
            }
            for level in levels {
                b.extend((0..4).rev().map(|i| (level >> i) & 1));
            }
            b.extend_from_slice(
                &original_bits[bit + old_matrix_bits..original_bits.len() - padding],
            );
            while b.len() % 8 != 0 {
                b.push(0);
            }
            let mut p = bytes(&b);
            let header_bytes = p.len();
            p.extend_from_slice(&frame.payload[old_header_bytes..]);
            intended.header_bytes = header_bytes;
            if Header::parse_intra(&intended_sequence, &p, frame.temporal_id, frame.spatial_id)
                .is_ok_and(|h| format!("{h:?}") == format!("{intended:?}"))
                && !matches.contains(&p)
            {
                matches.push(p);
            }
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "unique matrix-only frame rewrite, excluding required UV flag/alignment"
    );
    let frame_payload = matches.pop().unwrap();
    let mut output = Vec::new();
    let mut at = 0;
    for obu in &obus {
        let header = data[at];
        assert_eq!(header & 4, 0, "extension-free generator output");
        at += 1;
        loop {
            let byte = data[at];
            at += 1;
            if byte & 128 == 0 {
                break;
            }
        }
        at += obu.payload.len();
        let payload = if obu.kind == 1 {
            sequence_payload.as_slice()
        } else if obu.kind == 6 {
            frame_payload.as_slice()
        } else {
            obu.payload
        };
        output.push(header);
        let mut n = payload.len();
        loop {
            let v = (n & 127) as u8;
            n >>= 7;
            output.push(v | if n > 0 { 128 } else { 0 });
            if n == 0 {
                break;
            }
        }
        output.extend_from_slice(payload);
    }
    std::fs::write(&args[2], output).unwrap();
}
