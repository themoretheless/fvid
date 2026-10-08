/* Owned spatial/temporal-SVC fixture encoder. Generation-only stock libaom tool. */
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
  cfg.g_w=128; cfg.g_h=96; cfg.g_timebase.num=1; cfg.g_timebase.den=50;
  cfg.g_threads=1; cfg.g_lag_in_frames=0; cfg.g_error_resilient=1;
  cfg.rc_end_usage=AOM_CBR; cfg.rc_target_bitrate=1200; cfg.rc_min_quantizer=20; cfg.rc_max_quantizer=40;
  cfg.kf_max_dist=100;
  aom_codec_ctx_t ctx;
  check(aom_codec_enc_init(&ctx,aom_codec_av1_cx(),&cfg,0));
  check(aom_codec_control(&ctx,AOME_SET_CPUUSED,6));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_CDEF,0u));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_RESTORATION,0u));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_PALETTE,0));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_INTRABC,0));
  check(aom_codec_control(&ctx,AV1E_SET_ENABLE_REF_FRAME_MVS,0));
  aom_svc_params_t svc={0}; svc.number_spatial_layers=3; svc.number_temporal_layers=3;
  for (int sid=0;sid<3;sid++) {
    svc.scaling_factor_num[sid]=1; svc.scaling_factor_den[sid]=1<<(2-sid);
    for(int tid=0;tid<3;tid++) {
      int index=sid*3+tid;
      svc.max_quantizers[index]=40; svc.min_quantizers[index]=20;
      svc.layer_target_bitrate[index]=(100<<sid)*(tid+1);
    }
  }
  for(int tid=0;tid<3;tid++) svc.framerate_factor[tid]=1<<(2-tid);
  check(aom_codec_control(&ctx,AV1E_SET_SVC_PARAMS,&svc));
  aom_image_t *image=aom_img_alloc(NULL,AOM_IMG_FMT_I420,128,96,1); if(!image) return 2;

  const int tids[8]={0,2,1,2,0,2,1,2};
  for (int t=0;t<8;t++) {
    for (int p=0;p<3;p++) for(int y=0;y<(p?48:96);y++) for(int x=0;x<(p?64:128);x++)
      image->planes[p][y*image->stride[p]+x]=(unsigned char)(64+(x*3+y*5+t*7+p*23)%128);
    for (int sid=0;sid<3;sid++) {
      aom_svc_layer_id_t layer={sid,tids[t]}; check(aom_codec_control(&ctx,AV1E_SET_SVC_LAYER_ID,&layer));
      aom_svc_ref_frame_config_t refs={0};
      int tid=tids[t], base=sid*2;
      int destination=tid==0?base:tid==1?base+1:6+sid;
      for(int r=0;r<7;r++) refs.ref_idx[r]=base;
      if(tid!=2 || sid<2) {
        refs.ref_idx[1]=destination; refs.refresh[destination]=1;
      }
      if(t>0 && (sid==0 || tid==0)) refs.reference[0]=1;
      if(sid>0) {
        refs.ref_idx[3]=tid==0?(sid-1)*2:tid==1?(sid-1)*2+1:6+(sid-1);
        refs.reference[3]=1;
      }
      check(aom_codec_control(&ctx,AV1E_SET_SVC_REF_FRAME_CONFIG,&refs));
      check(aom_codec_encode(&ctx,image,t,1,t==0&&sid==0?AOM_EFLAG_FORCE_KF:0));
      aom_codec_iter_t iter=NULL; const aom_codec_cx_pkt_t *pkt;
      while((pkt=aom_codec_get_cx_data(&ctx,&iter))) if(pkt->kind==AOM_CODEC_CX_FRAME_PKT)
        if(fwrite(pkt->data.frame.buf,1,pkt->data.frame.sz,output)!=pkt->data.frame.sz) return 2;
    }
  }
  check(aom_codec_encode(&ctx,NULL,8,1,0));
  aom_codec_iter_t iter=NULL; const aom_codec_cx_pkt_t *pkt;
  while((pkt=aom_codec_get_cx_data(&ctx,&iter))) if(pkt->kind==AOM_CODEC_CX_FRAME_PKT)
    if(fwrite(pkt->data.frame.buf,1,pkt->data.frame.sz,output)!=pkt->data.frame.sz) return 2;
  aom_img_free(image); check(aom_codec_destroy(&ctx)); return fclose(output)?2:0;
}
