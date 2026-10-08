/* Original synthetic large-scale tile generation and stock libaom oracle only. */
#include <aom/aom_encoder.h>
#include <aom/aom_decoder.h>
#include <aom/aomcx.h>
#include <aom/aomdx.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static void checked(aom_codec_err_t e,int line){if(e!=AOM_CODEC_OK){fprintf(stderr,"aom at %d: %s\n",line,aom_codec_err_to_string(e));exit(1);}}
#define check(e) checked((e),__LINE__)
static void save(const char *prefix,const char *suffix,const void *data,size_t n){char path[1024];snprintf(path,sizeof(path),"%s%s",prefix,suffix);FILE *f=fopen(path,"wb");if(!f||fwrite(data,1,n,f)!=n||fclose(f))exit(2);}
static unsigned char *packet(aom_codec_ctx_t *ctx,size_t *n){aom_codec_iter_t it=NULL;const aom_codec_cx_pkt_t *p;while((p=aom_codec_get_cx_data(ctx,&it)))if(p->kind==AOM_CODEC_CX_FRAME_PKT){*n=p->data.frame.sz;fprintf(stderr,"packet pts=%lld key=%d size=%zu\n",(long long)p->data.frame.pts,!!(p->data.frame.flags&AOM_FRAME_IS_KEY),*n);unsigned char *out=malloc(*n);if(!out)exit(2);memcpy(out,p->data.frame.buf,*n);return out;}exit(2);}
static size_t leb(unsigned char *out,size_t n){size_t i=0;do{out[i++]=(n&127)|(n>127?128:0);n>>=7;}while(n);return i;}
int main(int argc,char **argv){
 if(argc!=2)return 2;const char *prefix=argv[1];
 aom_codec_enc_cfg_t cfg;check(aom_codec_enc_config_default(aom_codec_av1_cx(),&cfg,AOM_USAGE_REALTIME));
 cfg.g_w=128;cfg.g_h=128;cfg.g_timebase.num=1;cfg.g_timebase.den=50;cfg.g_threads=1;cfg.g_lag_in_frames=0;cfg.rc_end_usage=AOM_Q;cfg.rc_min_quantizer=0;cfg.rc_max_quantizer=0;cfg.kf_max_dist=100;cfg.kf_mode=AOM_KF_DISABLED;
 aom_codec_ctx_t enc;check(aom_codec_enc_init(&enc,aom_codec_av1_cx(),&cfg,0));
 check(aom_codec_control(&enc,AOME_SET_CPUUSED,6));check(aom_codec_control(&enc,AV1E_SET_LOSSLESS,1u));check(aom_codec_control(&enc,AV1E_SET_SUPERBLOCK_SIZE,AOM_SUPERBLOCK_SIZE_64X64));
 check(aom_codec_control(&enc,AV1E_SET_ENABLE_ORDER_HINT,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_REF_FRAME_MVS,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_CDEF,0u));check(aom_codec_control(&enc,AV1E_SET_ENABLE_RESTORATION,0u));check(aom_codec_control(&enc,AV1E_SET_LOOPFILTER_CONTROL,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_PALETTE,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_INTRABC,0));check(aom_codec_control(&enc,AV1E_SET_CDF_UPDATE_MODE,0u));
 aom_svc_params_t svc={0};svc.number_spatial_layers=1;svc.number_temporal_layers=1;svc.scaling_factor_num[0]=1;svc.scaling_factor_den[0]=1;svc.framerate_factor[0]=1;svc.layer_target_bitrate[0]=256;
 check(aom_codec_control(&enc,AV1E_SET_SVC_PARAMS,&svc));
 aom_image_t *img=aom_img_alloc(NULL,AOM_IMG_FMT_I420,128,128,1);if(!img)return 2;
 for(int p=0;p<3;p++)for(int y=0;y<(p?64:128);y++)for(int x=0;x<(p?64:128);x++)img->planes[p][y*img->stride[p]+x]=(64+3*x+5*y+23*p)%192;
 check(aom_codec_encode(&enc,img,0,1,AOM_EFLAG_FORCE_KF));size_t an;unsigned char *ab=packet(&enc,&an);save(prefix,"-anchor.obu",ab,an);
 cfg.large_scale_tile=1;check(aom_codec_enc_config_set(&enc,&cfg));check(aom_codec_control(&enc,AV1E_SET_TILE_COLUMNS,1u));check(aom_codec_control(&enc,AV1E_SET_TILE_ROWS,1u));check(aom_codec_control(&enc,AV1E_SET_SINGLE_TILE_DECODING,1u));
 check(aom_codec_encode(&enc,img,1,1,AOM_EFLAG_FORCE_KF));size_t prime_size;unsigned char *prime=packet(&enc,&prime_size);free(prime);
 av1_ref_frame_t forced={0};forced.idx=0;forced.img=*img;check(aom_codec_control(&enc,AV1_SET_REFERENCE,&forced));check(aom_codec_control(&enc,AV1E_SET_FRAME_PARALLEL_DECODING,1u));
 for(int p=0;p<3;p++)for(int y=0;y<(p?64:128);y++)for(int x=0;x<(p?64:128);x++)img->planes[p][y*img->stride[p]+x]=(71+3*x+5*y+23*p)%192;
 int flags=AOM_EFLAG_NO_REF_LAST2|AOM_EFLAG_NO_REF_LAST3|AOM_EFLAG_NO_REF_GF|AOM_EFLAG_NO_REF_ARF|AOM_EFLAG_NO_REF_BWD|AOM_EFLAG_NO_REF_ARF2|AOM_EFLAG_NO_UPD_LAST|AOM_EFLAG_NO_UPD_GF|AOM_EFLAG_NO_UPD_ARF|AOM_EFLAG_NO_UPD_ENTROPY;
 aom_svc_ref_frame_config_t refs={0};refs.reference[0]=1;
 check(aom_codec_control(&enc,AV1E_SET_SVC_REF_FRAME_CONFIG,&refs));
 check(aom_codec_encode(&enc,img,2,1,flags));size_t cn;unsigned char *cb=packet(&enc,&cn);save(prefix,"-camera.obu",cb,cn);
 aom_codec_ctx_t dec;check(aom_codec_dec_init(&dec,aom_codec_av1_dx(),NULL,0));check(aom_codec_decode(&dec,ab,an,NULL));
 check(aom_codec_control(&dec,AV1_SET_TILE_MODE,1u));check(aom_codec_control(&dec,AV1D_EXT_TILE_DEBUG,1u));check(aom_codec_control(&dec,AV1_SET_DECODE_TILE_ROW,0));check(aom_codec_control(&dec,AV1_SET_DECODE_TILE_COL,0));check(aom_codec_decode(&dec,cb,cn,NULL));
 aom_tile_data header={0};check(aom_codec_control(&dec,AV1D_GET_FRAME_HEADER_INFO,&header));size_t offset=(const unsigned char*)header.coded_tile_data-cb;size_t hs=header.extra_size-1;size_t hn=offset+header.coded_tile_data_size+hs;
 unsigned char *hb=malloc(hn);memcpy(hb,cb,hn);size_t n=hs;for(size_t i=0;i<header.coded_tile_data_size;i++){hb[offset+i]=(n&127)|(i+1<header.coded_tile_data_size?128:0);n>>=7;}save(prefix,"-header.obu",hb,hn);
 unsigned char payload[65536];size_t at=4;payload[0]=1;payload[1]=1;payload[2]=0;payload[3]=3;
 const int order[4]={3,0,2,1};
 for(int i=0;i<4;i++){int tile=order[i];check(aom_codec_control(&dec,AV1_SET_DECODE_TILE_ROW,tile/2));check(aom_codec_control(&dec,AV1_SET_DECODE_TILE_COL,tile%2));check(aom_codec_decode(&dec,cb,cn,NULL));aom_tile_data td={0};check(aom_codec_control(&dec,AV1D_GET_TILE_DATA,&td));if(!td.coded_tile_data_size||at+5+td.coded_tile_data_size>sizeof(payload))return 2;payload[at++]=0;payload[at++]=tile/2;payload[at++]=tile%2;payload[at++]=(td.coded_tile_data_size-1)>>8;payload[at++]=(td.coded_tile_data_size-1)&255;memcpy(payload+at,td.coded_tile_data,td.coded_tile_data_size);at+=td.coded_tile_data_size;}
 unsigned char list[65548];list[0]=0x42;size_t ln=1+leb(list+1,at);memcpy(list+ln,payload,at);ln+=at;save(prefix,"-list.obu",list,ln);
 aom_codec_ctx_t oracle;check(aom_codec_dec_init(&oracle,aom_codec_av1_dx(),NULL,0));check(aom_codec_decode(&oracle,ab,an,NULL));
 aom_img_fmt_t format=0;check(aom_codec_control(&oracle,AV1D_GET_IMG_FORMAT,&format));
 aom_image_t anchor; if(!aom_img_alloc_with_border(&anchor,format,128,128,32,8,64))return 2;
 check(aom_codec_control(&oracle,AV1_COPY_NEW_FRAME_IMAGE,&anchor));
 check(aom_codec_control(&oracle,AV1_SET_TILE_MODE,1u));av1_ext_ref_frame_t external={&anchor,1};check(aom_codec_control(&oracle,AV1D_SET_EXT_REF_PTR,&external));
 check(aom_codec_decode(&oracle,hb,hn,NULL));
 aom_codec_err_t status=aom_codec_decode(&oracle,list,ln,NULL);if(status!=AOM_CODEC_OK)fprintf(stderr,"oracle detail: %s\n",aom_codec_error_detail(&oracle));check(status);
 aom_codec_iter_t it=NULL;aom_image_t *out=aom_codec_get_frame(&oracle,&it);if(!out||out->d_w!=128||out->d_h!=128)return 2;
 unsigned char golden[128*128*3/2];size_t pos=0;
 for(int p=0;p<3;p++)for(int y=0;y<(p?64:128);y++){size_t width=p?64:128;memcpy(golden+pos,out->planes[p]+y*out->stride[p],width);pos+=width;}
 save(prefix,"-list.yuv",golden,pos);aom_img_free(&anchor);check(aom_codec_destroy(&oracle));
 free(hb);free(ab);free(cb);aom_img_free(img);check(aom_codec_destroy(&enc));check(aom_codec_destroy(&dec));return 0;
}
