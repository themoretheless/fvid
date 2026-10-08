/* Owned temporal-SVC fixture encoder. Generation-only stock libaom tool. */
#include <aom/aom_encoder.h>
#include <aom/aomcx.h>
#include <stdio.h>
#include <stdlib.h>
static void check(aom_codec_err_t e) {
  if (e != AOM_CODEC_OK) { fprintf(stderr,"aom: %s\n",aom_codec_err_to_string(e)); exit(1); }
}
int main(int argc, char **argv) {
  if (argc != 2) return 2;
  FILE *output = fopen(argv[1],"wb"); if (!output) return 2;
  aom_codec_enc_cfg_t cfg;
  check(aom_codec_enc_config_default(aom_codec_av1_cx(), &cfg, AOM_USAGE_REALTIME));
  cfg.g_w=64; cfg.g_h=48; cfg.g_timebase.num=1; cfg.g_timebase.den=50;
  cfg.g_threads=1; cfg.g_lag_in_frames=0; cfg.g_error_resilient=1;
  cfg.rc_end_usage=AOM_CBR; cfg.rc_target_bitrate=300; cfg.rc_min_quantizer=20; cfg.rc_max_quantizer=40;
  cfg.kf_max_dist=100;
  aom_codec_ctx_t ctx;
  check(aom_codec_enc_init(&ctx,aom_codec_av1_cx(),&cfg,0));
  check(aom_codec_control(&ctx,AOME_SET_CPUUSED,6));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_CDEF,0u));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_RESTORATION,0u));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_PALETTE,0));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_INTRABC,0));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_REF_FRAME_MVS,0));
  aom_svc_params_t svc={0}; svc.number_spatial_layers=1; svc.number_temporal_layers=3;
  svc.scaling_factor_num[0]=1; svc.scaling_factor_den[0]=1;
  for (int i=0;i<3;i++) { svc.max_quantizers[i]=40; svc.min_quantizers[i]=20; svc.layer_target_bitrate[i]=100*(i+1); svc.framerate_factor[i]=1<<(2-i); }
  check(aom_codec_control(&ctx,AV1E_SET_SVC_PARAMS,&svc));
  aom_image_t *image=aom_img_alloc(NULL,AOM_IMG_FMT_I420,64,48,1); if(!image) return 2;
  const int tids[8]={0,2,1,2,0,2,1,2};
  for (int t=0;t<8;t++) {
    for (int p=0;p<3;p++) for(int y=0;y<(p?24:48);y++) for(int x=0;x<(p?32:64);x++)
      image->planes[p][y*image->stride[p]+x]=(unsigned char)(64+(x*3+y*5+t*7+p*23)%128);
    aom_svc_layer_id_t layer={0,tids[t]}; check(aom_codec_control(&ctx,AV1E_SET_SVC_LAYER_ID,&layer));
    aom_svc_ref_frame_config_t refs={0}; refs.reference[0]=1;
    for(int r=0;r<7;r++) refs.ref_idx[r]=0;
    refs.ref_idx[1]=tids[t]; refs.refresh[tids[t]]=1;
    check(aom_codec_control(&ctx,AV1E_SET_SVC_REF_FRAME_CONFIG,&refs));
    check(aom_codec_encode(&ctx,image,t,1,t==0?AOM_EFLAG_FORCE_KF:0));
    aom_codec_iter_t iter=NULL; const aom_codec_cx_pkt_t *pkt;
    while((pkt=aom_codec_get_cx_data(&ctx,&iter))) if(pkt->kind==AOM_CODEC_CX_FRAME_PKT)
      if(fwrite(pkt->data.frame.buf,1,pkt->data.frame.sz,output)!=pkt->data.frame.sz) return 2;
  }
  check(aom_codec_encode(&ctx,NULL,8,1,0));
  aom_codec_iter_t iter=NULL; const aom_codec_cx_pkt_t *pkt;
  while((pkt=aom_codec_get_cx_data(&ctx,&iter))) if(pkt->kind==AOM_CODEC_CX_FRAME_PKT)
    if(fwrite(pkt->data.frame.buf,1,pkt->data.frame.sz,output)!=pkt->data.frame.sz) return 2;
  aom_img_free(image); check(aom_codec_destroy(&ctx)); return fclose(output)?2:0;
}
