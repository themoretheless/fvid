use fvid::codec::{
    avc_transform::{inverse_scan_4x4, reconstruct_4x4, residual_4x4},
    bits::BitReader,
    cavlc::read_residual,
};

#[test]
fn compressed_cavlc_bits_reconstruct_a_known_pixel_block() {
    // nC=0: token=(1,0):000101, first non-trailing level=+2:1, total_zeros=0:1.
    let mut bits = BitReader::new(&[0x17]);
    let block = read_residual(&mut bits, 0, 16).unwrap();
    let raster = inverse_scan_4x4(&block.coefficients, false);
    let residual = residual_4x4(&raster, 24, 8, &[16; 16], None).unwrap();
    let pixels = reconstruct_4x4(&[100; 16], &residual, 8).unwrap();
    assert_eq!(pixels, [105; 16]);
    assert_eq!(bits.position(), 8);
}
