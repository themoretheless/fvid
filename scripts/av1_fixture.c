/* Synthetic native-AV1 test input generator. libaom is an external oracle only. */
#include <aom/aom_encoder.h>
#include <aom/aomcx.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static void check(aom_codec_err_t e) { if(e != AOM_CODEC_OK) { fprintf(stderr,"aom: %s\n",aom_codec_err_to_string(e)); exit(1); } }
int main(int argc, char **argv) {
  int size = argc>1?atoi(argv[1]):32;
  int pattern = argc>2?atoi(argv[2]):0;
  int q = argc>3?atoi(argv[3]):0;
  int full = argc>4?atoi(argv[4]):0;
  int frames=argc>5?atoi(argv[5]):1;
  int depth=argc>6?atoi(argv[6]):8;
  int filters=argc>7?atoi(argv[7]):0;
  int tilecols=argc>8?atoi(argv[8]):0;
  int inter=argc>9?atoi(argv[9]):0;
  aom_codec_enc_cfg_t cfg;
  check(aom_codec_enc_config_default(aom_codec_av1_cx(), &cfg, AOM_USAGE_GOOD_QUALITY));
  cfg.g_w=size; cfg.g_h=size; cfg.g_timebase.num=1; cfg.g_timebase.den=3;
  cfg.g_threads=1; cfg.g_lag_in_frames=0; cfg.rc_end_usage=AOM_Q;
  cfg.rc_min_quantizer=q; cfg.rc_max_quantizer=q; cfg.kf_max_dist=inter?100:1;
  cfg.g_bit_depth=depth;cfg.g_input_bit_depth=depth;cfg.g_profile=depth==12?2:0;
  aom_codec_ctx_t ctx;
  check(aom_codec_enc_init(&ctx,aom_codec_av1_cx(),&cfg,depth>8?AOM_CODEC_USE_HIGHBITDEPTH:0));
  check(aom_codec_control(&ctx,AOME_SET_CPUUSED,6));
  check(aom_codec_control(&ctx,AV1E_SET_LOSSLESS,(unsigned)(q==0)));
  check(aom_codec_control(&ctx,AV1E_SET_AQ_MODE,0u));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_CDEF,(unsigned)filters));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_RESTORATION,0u));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_PALETTE,0));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_INTRABC,0));
  check(aom_codec_control(&ctx,AV1E_SET_LOOPFILTER_CONTROL,filters));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_DIRECTIONAL_INTRA,full));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_SMOOTH_INTRA,full));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_PAETH_INTRA,full));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_CFL_INTRA,full));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_FILTER_INTRA,full));
  check(aom_codec_control(&ctx,AV1E_SET_TILE_COLUMNS,tilecols));
  if(inter) {
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_REF_FRAME_MVS,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_GLOBAL_MOTION,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_WARPED_MOTION,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_OBMC,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_INTERINTRA_COMP,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_RECT_PARTITIONS,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_AB_PARTITIONS,0));
    check(aom_codec_control(&ctx,AV1E_SET_ENABLE_1TO4_PARTITIONS,0));
  }
  aom_image_t *img=aom_img_alloc(NULL,depth>8?AOM_IMG_FMT_I42016:AOM_IMG_FMT_I420,size,size,1);
  if(!img) return 1;
  for(int frame=0;frame<frames;frame++) {
    for(int p=0;p<3;p++) for(int y=0;y<(p?(size+1)/2:size);y++) for(int x=0;x<(p?(size+1)/2:size);x++) {
      unsigned value=pattern ? (x*7+y*3+p*59+frame*23)%256 : 128;
      value=value<<(depth-8);
      if(depth==8) img->planes[p][y*img->stride[p]+x]=value;
      else ((uint16_t*)(img->planes[p]+y*img->stride[p]))[x]=value;
    }
    check(aom_codec_encode(&ctx,img,frame,1,inter&&frame?0:AOM_EFLAG_FORCE_KF));
    aom_codec_iter_t iter=NULL; const aom_codec_cx_pkt_t *pkt;
    while((pkt=aom_codec_get_cx_data(&ctx,&iter))) if(pkt->kind==AOM_CODEC_CX_FRAME_PKT)
      fwrite(pkt->data.frame.buf,1,pkt->data.frame.sz,stdout);
  }
  aom_img_free(img); check(aom_codec_destroy(&ctx));
  return ferror(stdout)?1:0;
}
