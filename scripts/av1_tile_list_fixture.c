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
static unsigned sample(const aom_image_t *img,int p,int x,int y){return img->fmt&AOM_IMG_FMT_HIGHBITDEPTH?((const unsigned short*)(img->planes[p]+y*img->stride[p]))[x]:img->planes[p][y*img->stride[p]+x];}
static void put(aom_image_t *img,int p,int x,int y,unsigned value){if(img->fmt&AOM_IMG_FMT_HIGHBITDEPTH)((unsigned short*)(img->planes[p]+y*img->stride[p]))[x]=value;else img->planes[p][y*img->stride[p]+x]=value;}
static void save_image(const char *prefix,const char *suffix,const aom_image_t *img,int depth){size_t capacity=(size_t)img->d_w*img->d_h*6;unsigned char *raw=malloc(capacity);if(!raw)exit(2);size_t at=0;for(int p=0;p<3;p++)for(unsigned y=0;y<(img->d_h>>(p?img->y_chroma_shift:0));y++)for(unsigned x=0;x<(img->d_w>>(p?img->x_chroma_shift:0));x++){unsigned v=sample(img,p,x,y);raw[at++]=v&255;if(depth>8)raw[at++]=v>>8;}save(prefix,suffix,raw,at);free(raw);}
int main(int argc,char **argv){
 if(argc!=2&&argc!=3&&argc!=5&&argc!=6)return 2;const char *prefix=argv[1];int sb=argc>=3?atoi(argv[2]):64;int depth=argc>=5?atoi(argv[3]):8;int chroma=argc>=5?atoi(argv[4]):420;int q=argc==6?atoi(argv[5]):0;
 if(q<0||q>63)return 2;
 if((sb!=64&&sb!=128)||(depth!=8&&depth!=10&&depth!=12)||(chroma!=420&&chroma!=422&&chroma!=444))return 2;int dimension=2*sb;int sx=chroma==444?0:1;int sy=chroma==420?1:0;int shift=depth-8;
 aom_img_fmt_t input_format=chroma==420?AOM_IMG_FMT_I420:chroma==422?AOM_IMG_FMT_I422:AOM_IMG_FMT_I444;if(depth>8)input_format|=AOM_IMG_FMT_HIGHBITDEPTH;
 aom_codec_enc_cfg_t cfg;check(aom_codec_enc_config_default(aom_codec_av1_cx(),&cfg,AOM_USAGE_REALTIME));
 cfg.g_w=dimension;cfg.g_h=dimension;cfg.g_timebase.num=1;cfg.g_timebase.den=50;cfg.g_threads=1;cfg.g_lag_in_frames=0;cfg.rc_end_usage=AOM_Q;cfg.rc_min_quantizer=0;cfg.rc_max_quantizer=0;cfg.kf_max_dist=100;cfg.kf_mode=AOM_KF_DISABLED;
 cfg.g_bit_depth=(aom_bit_depth_t)depth;cfg.g_input_bit_depth=depth;cfg.g_profile=depth==12||chroma==422?2:chroma==444?1:0;
 aom_codec_ctx_t enc;check(aom_codec_enc_init(&enc,aom_codec_av1_cx(),&cfg,depth>8?AOM_CODEC_USE_HIGHBITDEPTH:0));
 check(aom_codec_control(&enc,AOME_SET_CPUUSED,6));check(aom_codec_control(&enc,AV1E_SET_LOSSLESS,1u));check(aom_codec_control(&enc,AV1E_SET_SUPERBLOCK_SIZE,sb==128?AOM_SUPERBLOCK_SIZE_128X128:AOM_SUPERBLOCK_SIZE_64X64));
 check(aom_codec_control(&enc,AV1E_SET_ENABLE_ORDER_HINT,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_REF_FRAME_MVS,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_CDEF,0u));check(aom_codec_control(&enc,AV1E_SET_ENABLE_RESTORATION,0u));check(aom_codec_control(&enc,AV1E_SET_LOOPFILTER_CONTROL,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_PALETTE,0));check(aom_codec_control(&enc,AV1E_SET_ENABLE_INTRABC,0));check(aom_codec_control(&enc,AV1E_SET_CDF_UPDATE_MODE,0u));
 aom_svc_params_t svc={0};svc.number_spatial_layers=1;svc.number_temporal_layers=1;svc.scaling_factor_num[0]=1;svc.scaling_factor_den[0]=1;svc.framerate_factor[0]=1;svc.layer_target_bitrate[0]=256;
 check(aom_codec_control(&enc,AV1E_SET_SVC_PARAMS,&svc));
 aom_image_t *img=aom_img_alloc(NULL,input_format,dimension,dimension,1);if(!img)return 2;
 for(int p=0;p<3;p++)for(int y=0;y<(dimension>>(p?sy:0));y++)for(int x=0;x<(dimension>>(p?sx:0));x++)put(img,p,x,y,(64+(3*x+5*y+23*p)%96)<<shift);
 check(aom_codec_encode(&enc,img,0,1,AOM_EFLAG_FORCE_KF));size_t an;unsigned char *ab=packet(&enc,&an);save(prefix,"-anchor.obu",ab,an);
 cfg.large_scale_tile=1;check(aom_codec_enc_config_set(&enc,&cfg));check(aom_codec_control(&enc,AV1E_SET_TILE_COLUMNS,1u));check(aom_codec_control(&enc,AV1E_SET_TILE_ROWS,1u));check(aom_codec_control(&enc,AV1E_SET_SINGLE_TILE_DECODING,1u));
 check(aom_codec_encode(&enc,img,1,1,AOM_EFLAG_FORCE_KF));size_t prime_size;unsigned char *prime=packet(&enc,&prime_size);free(prime);
 av1_ref_frame_t forced={0};forced.idx=0;forced.img=*img;check(aom_codec_control(&enc,AV1_SET_REFERENCE,&forced));check(aom_codec_control(&enc,AV1E_SET_FRAME_PARALLEL_DECODING,1u));
 if(q){check(aom_codec_control(&enc,AV1E_SET_LOSSLESS,0u));check(aom_codec_control(&enc,AV1E_SET_QUANTIZER_ONE_PASS,q));}
 for(int p=0;p<3;p++)for(int y=0;y<(dimension>>(p?sy:0));y++)for(int x=0;x<(dimension>>(p?sx:0));x++)put(img,p,x,y,(71+(3*x+5*y+23*p)%96+(q?((x/8+y/8+p)%7-3):0))<<shift);
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
 for(int i=0;i<4;i++){int tile=order[i];check(aom_codec_control(&dec,AV1_SET_DECODE_TILE_ROW,tile/2));check(aom_codec_control(&dec,AV1_SET_DECODE_TILE_COL,tile%2));check(aom_codec_decode(&dec,cb,cn,NULL));aom_codec_iter_t tile_it=NULL;aom_image_t *tile_image=aom_codec_get_frame(&dec,&tile_it);if(!tile_image||tile_image->d_w!=(unsigned)sb||tile_image->d_h!=(unsigned)sb)return 2;char tile_suffix[32];snprintf(tile_suffix,sizeof(tile_suffix),"-tile%d.yuv",i);save_image(prefix,tile_suffix,tile_image,depth);aom_tile_data td={0};check(aom_codec_control(&dec,AV1D_GET_TILE_DATA,&td));if(!td.coded_tile_data_size||at+5+td.coded_tile_data_size>sizeof(payload))return 2;payload[at++]=0;payload[at++]=tile/2;payload[at++]=tile%2;payload[at++]=(td.coded_tile_data_size-1)>>8;payload[at++]=(td.coded_tile_data_size-1)&255;memcpy(payload+at,td.coded_tile_data,td.coded_tile_data_size);at+=td.coded_tile_data_size;}
 unsigned char list[65548];list[0]=0x42;size_t ln=1+leb(list+1,at);memcpy(list+ln,payload,at);ln+=at;save(prefix,"-list.obu",list,ln);
 aom_codec_ctx_t oracle;check(aom_codec_dec_init(&oracle,aom_codec_av1_dx(),NULL,0));check(aom_codec_decode(&oracle,ab,an,NULL));
 aom_img_fmt_t format=0;check(aom_codec_control(&oracle,AV1D_GET_IMG_FORMAT,&format));
 aom_image_t anchor; if(!aom_img_alloc_with_border(&anchor,format,dimension,dimension,32,8,64))return 2;
 check(aom_codec_control(&oracle,AV1_COPY_NEW_FRAME_IMAGE,&anchor));
 check(aom_codec_control(&oracle,AV1_SET_TILE_MODE,1u));av1_ext_ref_frame_t external={&anchor,1};check(aom_codec_control(&oracle,AV1D_SET_EXT_REF_PTR,&external));
 check(aom_codec_decode(&oracle,hb,hn,NULL));
 aom_codec_err_t status=aom_codec_decode(&oracle,list,ln,NULL);if(status!=AOM_CODEC_OK)fprintf(stderr,"oracle detail: %s\n",aom_codec_error_detail(&oracle));check(status);
 aom_codec_iter_t it=NULL;aom_image_t *out=aom_codec_get_frame(&oracle,&it);if(!out||out->d_w!=(unsigned)dimension||out->d_h!=(unsigned)dimension)return 2;
 save_image(prefix,"-list.yuv",out,depth);
 /* A second, distinct owned external anchor proves per-entry anchor selection. */
 aom_image_t alternate;if(!aom_img_alloc_with_border(&alternate,format,dimension,dimension,32,8,64))return 2;
 for(int p=0;p<3;p++)for(int y=0;y<(dimension>>(p?sy:0));y++)for(int x=0;x<(dimension>>(p?sx:0));x++)put(&alternate,p,x,y,sample(&anchor,p,x,y)+(9<<shift));
 aom_image_t external_images[2]={anchor,alternate};external=(av1_ext_ref_frame_t){external_images,2};
 check(aom_codec_control(&oracle,AV1D_SET_EXT_REF_PTR,&external));
 size_t entry_at=4;
 for(int i=0;i<4;i++){payload[entry_at]=i%2;size_t length=((size_t)payload[entry_at+3]<<8)+payload[entry_at+4]+1;entry_at+=5+length;}
 memcpy(list+ln-at,payload,at);save(prefix,"-multi-list.obu",list,ln);
 check(aom_codec_decode(&oracle,hb,hn,NULL));check(aom_codec_decode(&oracle,list,ln,NULL));
 it=NULL;out=aom_codec_get_frame(&oracle,&it);if(!out||out->d_w!=(unsigned)dimension||out->d_h!=(unsigned)dimension)return 2;
 save_image(prefix,"-multi-list.yuv",out,depth);aom_img_free(&alternate);aom_img_free(&anchor);check(aom_codec_destroy(&oracle));
 free(hb);free(ab);free(cb);aom_img_free(img);check(aom_codec_destroy(&enc));check(aom_codec_destroy(&dec));return 0;
}
