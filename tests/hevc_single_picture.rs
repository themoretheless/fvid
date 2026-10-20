use fvid::{
    codec::{hevc_3d_syntax::SpsExtension, hevc_decoder::HevcDecoder, hevc_picture},
    container::mp4::Mp4Reader,
};
#[test]
fn single_ctu_entropy_dispatch_reconstructs_complete_picture() {
    let mut movie = Mp4Reader::open(
        std::io::Cursor::new(include_bytes!("fixtures/hevc/main-ipb.mp4")),
        Default::default(),
    )
    .unwrap();
    let decoder =
        HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
    let mut packet = Vec::new();
    movie.read_packet(0, 0, &mut packet).unwrap();
    let baseline = decoder.slice_headers(&packet).unwrap().remove(0);
    let (source_sps, source_pps) = decoder.parameters();
    let mut sps = source_sps.clone();
    let mut pps = source_pps.clone();
    sps.dimensions = [64, 64];
    sps.crop = [0; 4];
    sps.chroma_format = 0;
    sps.coding_block_log2 = [3, 6];
    sps.sao = false;
    sps.depth_layer = true;
    sps.three_d = Some(SpsExtension {
        inter_view_disparity: [false; 2],
        inter_view_mv_scaling: [false; 2],
        disparity_sub_block_log2: 3,
        inter_view_residual_prediction: false,
        depth_reference: false,
        view_synthesis_prediction: false,
        depth_based_partition: false,
        texture_motion: false,
        texture_sub_block_log2: 3,
        intra_contour: false,
        intra_dc_wedge: false,
        coding_tree_partition: false,
        inter_dc_only: false,
        skip_intra: true,
    });
    pps.entropy_sync = false;
    pps.tiles = None;
    pps.transquant_bypass = false;
    pps.cu_qp_delta_depth = None;
    for (size, suffix) in [(64, ""), (128, "-128")] {
        sps.dimensions = [size, size];
        for mode in 0..4 {
            let mut slice = baseline.clone();
            slice.qp = 26;
            slice.sao = [false; 2];
            slice.deblocking.disabled = true;
            slice.rbsp = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                format!("tests/fixtures/playback-errors/hevc-wedge/single-cu-{mode}{suffix}.cabac"),
            ))
            .unwrap();
            slice.entropy_byte_offset = 0;
            slice.entropy_substreams = vec![0..slice.rbsp.len()];
            slice.entry_point_offsets.clear();
            let picture = hevc_picture::decode_idr(&sps, &pps, &slice, 32 << 20).unwrap();
            assert_eq!(picture.dimensions, [size, size]);
            assert!(picture.planes[0].complete());
            assert!(picture.planes[0].samples().iter().all(|&v| v == 128));
        }
    }
}
