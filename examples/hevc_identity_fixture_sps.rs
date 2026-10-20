//! Offline SPS authoring helper: only VUI changes, all other parsed parameters match.
use fvid::codec::{bits::BitReader, hevc_nal::NalRbsp, hevc_sps::Sps, hevc_vui::Vui};
fn pack(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0; bits.len().div_ceil(8)];
    for (i, &b) in bits.iter().enumerate() {
        out[i / 8] |= u8::from(b) << (7 - i % 8);
    }
    out
}
fn escape(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut zeros = 0;
    for &v in bytes {
        if zeros == 2 && v <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(v);
        zeros = if v == 0 { zeros + 1 } else { 0 };
    }
    out
}
fn bits(bytes: &[u8]) -> Vec<bool> {
    bytes
        .iter()
        .flat_map(|v| (0..8).rev().map(move |n| v & (1 << n) != 0))
        .collect()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let nal = std::fs::read(&args[1])?;
    let full = args[2] == "full";
    let original = Sps::parse(&nal, 1 << 20)?;
    let rbsp = NalRbsp::parse(&nal, 1 << 20)?.bytes;
    let mut source = bits(&rbsp);
    source.truncate(source.iter().rposition(|b| *b).ok_or("missing stop bit")? + 1);
    let mut syntax = vec![false, false, true, true, false, true, full, true];
    for v in [1u8, 13, 0] {
        syntax.extend((0..8).rev().map(|n| v & (1 << n) != 0));
    }
    syntax.extend([false; 7]);
    let mut wanted = original.clone();
    wanted.vui = None;
    let mut found = Vec::new();
    for start in 0..source.len() {
        let end = if let Some(vui) = &original.vui {
            if !source[start] {
                continue;
            }
            let mut reader = BitReader::new(&rbsp);
            while reader.position() < start + 1 {
                let n = (start + 1 - reader.position()).min(32);
                reader.read(n as u8)?;
            }
            let Ok(parsed) = Vui::read(&mut reader, (original.ordering.len() - 1) as u8) else {
                continue;
            };
            if &parsed != vui {
                continue;
            }
            reader.position()
        } else {
            if source[start] {
                continue;
            }
            start + 1
        };
        if end > source.len() {
            continue;
        }
        let mut candidate = source[..start].to_vec();
        candidate.push(true);
        candidate.extend_from_slice(&syntax);
        candidate.extend_from_slice(&source[end..]);
        let mut encoded = nal[..2].to_vec();
        encoded.extend(escape(&pack(&candidate)));
        let Ok(mut parsed) = Sps::parse(&encoded, 1 << 20) else {
            continue;
        };
        if !parsed
            .vui
            .as_ref()
            .and_then(|v| v.signal.as_ref())
            .is_some_and(|v| v.full_range == full && v.colour == Some([1, 13, 0]))
        {
            continue;
        }
        parsed.vui = None;
        if parsed == wanted {
            found.push(encoded);
        }
    }
    if found.len() != 1 {
        return Err(format!(
            "expected one semantic-preserving VUI rewrite, got {}",
            found.len()
        )
        .into());
    }
    std::fs::write(&args[3], &found[0])?;
    Ok(())
}
