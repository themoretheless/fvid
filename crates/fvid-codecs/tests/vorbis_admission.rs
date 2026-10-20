use fvid_codecs::vorbis::{Identification, Setup};
use fvid_codecs::vorbis_admission::inspect;
#[test]
fn preflight_admits_actual_synthetic_setups_before_table_allocation() {
    for bytes in [
        include_bytes!("../../../tests/fixtures/ogg/vorbis-stereo.ogg").as_slice(),
        include_bytes!(
            "../../../tests/fixtures/playback-errors/vorbis-timestamps-reconfigured.ogg"
        )
        .as_slice(),
    ] {
        let ogg = fvid_media::owned_ogg::Ogg::parse(bytes, &Default::default()).unwrap();
        for chain in ogg.chains() {
            let header = &chain.packets[2].data;
            let admission = inspect(header).unwrap();
            let id = Identification::parse(&chain.packets[0].data).unwrap();
            let setup = Setup::parse(header, id.channels, admission.limits).unwrap();
            assert!(admission.bytes > header.len());
            assert!(admission.max_dimension > 0);
            assert!(!setup.codebooks.is_empty());
            let mut limits = admission.limits;
            limits.entries -= 1;
            assert!(Setup::parse(header, id.channels, limits).is_err());
            limits = admission.limits;
            limits.nodes -= 1;
            assert!(Setup::parse(header, id.channels, limits).is_err());
            for end in 0..8 {
                assert!(inspect(&header[..end]).is_err());
            }
        }
    }
}

fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    for &(value, width) in fields {
        for shift in 0..width {
            if at % 8 == 0 {
                out.push(0);
            }
            out[at / 8] |= (((value >> shift) & 1) as u8) << (at % 8);
            at += 1;
        }
    }
    out
}
#[test]
fn compressed_ordered_and_sparse_books_charge_expanded_geometry() {
    use fvid_codecs::vorbis::{Codebook, PacketBits};
    let mut prefix = vec![(5, 8)];
    prefix.extend(b"vorbis".iter().map(|&b| (u32::from(b), 8)));
    prefix.extend([(0, 8), (0x564342, 24), (1, 16), (4096, 24)]);
    for sparse in [false, true] {
        let mut fields = prefix.clone();
        if sparse {
            fields.extend([(0, 1), (1, 1), (1, 1), (0, 5), (1, 1), (0, 5)]);
            fields.extend(vec![(0, 1); 4094]);
        } else {
            fields.extend([(1, 1), (11, 5), (4096, 13)]);
        }
        fields.push((0, 4));
        let bytes = pack(&fields);
        let admission = inspect(&bytes).unwrap();
        assert_eq!(admission.limits.entries, 4096);
        assert_eq!(admission.limits.nodes, if sparse { 3 } else { 8191 });
        assert_eq!(admission.limits.multiplicands, 0);
        let mut bits = PacketBits::new(&bytes);
        bits.skip(64).unwrap();
        Codebook::parse(
            &mut bits,
            admission.limits.entries,
            admission.limits.nodes,
            0,
        )
        .unwrap();
        assert!(admission.bytes > bytes.len() * 100);
    }
}
