/* Research harness: unmodified externally supplied ISO spatial frontend.
   AFILE adapter writes raw normalized float PCM; no codec is stubbed.
   Source-specific admission: mono 48kHz, eight slots, 64 QMF bands. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <pthread.h>
#include <setjmp.h>
#include "sac_dec.h"
#include "sac_polyphase.h"
int debug[256];
long frameCounter=0;
Twiddle *twiddleExtern=0;
struct AFILE {FILE*f;};
AFILE *AFopnWrite(const char*p,int type,int bits,long channels,double rate,void*x){AFILE*a=calloc(1,sizeof(*a));a->f=fopen(p,"wb");if(!a->f)exit(8);return a;}
void AFclose(AFILE*a){fclose(a->f);free(a);}
long AFfWriteData(AFILE*a,const float*v,long n){if(fwrite(v,4,n,a->f)!=(size_t)n)exit(9);return n;}
void myexit(char*s){fprintf(stderr,"ISO_ERROR:%s\n",s);exit(10);}
struct Reader{BYTE_READER base;unsigned char *data;size_t at,size;};
static int next(HANDLE_BYTE_READER r){struct Reader*v=(struct Reader*)r;return v->at<v->size?v->data[v->at++]:-1;}
static uint32_t word(FILE*f){uint32_t v;if(fread(&v,4,1,f)!=1)exit(11);return v;}
static void*run(void*arg){char**argv=arg;FILE*cfg=fopen(argv[1],"rb"),*frames=fopen(argv[2],"rb"),*core=fopen(argv[3],"rb");if(!cfg||!frames||!core)exit(2);unsigned char header[65536];size_t size=fread(header,1,sizeof(header),cfg);struct Reader r={{next},header,0,size};int slots=0;
spatialDec*s=SpatialDecOpen(&r.base,argv[4],24,48000,1,&slots,64,atoi(argv[5]),0,0,NULL,0);if(!s)exit(3);fprintf(stderr,"slots=%d bands=%d channels=%d outputs=%d frameLength=%d\n",slots,s->qmfBands,s->numInputChannels,s->numOutputChannelsAT,s->frameLength);if(slots!=8||s->qmfBands!=64||s->numInputChannels!=1||s->numOutputChannelsAT!=6)exit(4);
SacInitAnaFilterbank(NULL,64);SAC_POLYPHASE_ANA_FILTERBANK*analysis;SacOpenAnaFilterbank(&analysis);uint32_t count=word(frames);float pcm[512],in[64],re[8][64],im[8][64],*rr[8],*ii[8],**pointers[4]={rr,ii,0,0};for(int t=0;t<8;t++){rr[t]=re[t];ii[t]=im[t];}
for(uint32_t frame=0;frame<count;frame++){uint32_t n=word(frames);unsigned char*data=malloc(n);if(fread(data,1,n,frames)!=n||fread(pcm,4,512,core)!=512)exit(5);s_bitinput_close(s->bitstream);r.data=data;r.size=n;r.at=0;s->bitstream=s_bitinput_open(&r.base);SpatialDecParseFrame(s);for(int t=0;t<8;t++){for(int q=0;q<64;q++)in[q]=pcm[t*64+q]*32768.f*SpatialGetClipProtectGain(s);SacCalculateAnaFilterbank(analysis,in,re[t],im[t]);}SpatialDecApplyFrame(s,0,8,pointers);free(data);if(frame%500==0)fprintf(stderr,"frame %u\n",frame);}
if(fgetc(frames)!=EOF||fgetc(core)!=EOF)exit(13);
SacCloseAnaFilterbank(analysis);SpatialDecClose(s);fclose(cfg);fclose(frames);fclose(core);return NULL;}
int main(int argc,char**argv){if(argc!=6)return 1;pthread_t t;pthread_attr_t a;pthread_attr_init(&a);pthread_attr_setstacksize(&a,64*1024*1024);if(pthread_create(&t,&a,run,argv)||pthread_join(t,NULL))return 12;return 0;}
