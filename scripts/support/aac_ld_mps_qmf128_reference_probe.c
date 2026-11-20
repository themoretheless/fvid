/* Explicit ordinary-output component chain, original ISO apply/decor/merge.
   The unselected HRTF branch aborts rather than supplying an implementation. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>
#include <math.h>
#include <string.h>
#include "sac_dec.h"
#include "sac_process.h"
#include "sac_decor.h"
#include "sac_calcM1andM2.h"
#include "sac_hrtf.h"
int debug[256];
void myexit(char*s){fprintf(stderr,"%s\n",s);exit(90);}
void SpatialDecApplyHrtfFilterbank(HRTF_FILTERBANK*self,float a[][MAX_TIME_SLOTS][MAX_HYBRID_BANDS],float b[][MAX_TIME_SLOTS][MAX_HYBRID_BANDS],float c[][MAX_TIME_SLOTS][MAX_HYBRID_BANDS],float d[][MAX_TIME_SLOTS][MAX_HYBRID_BANDS],int e){abort();}
static int get(FILE*f,void*v,size_t n){return fread(v,1,n,f)==n;}
static float sample(int residual,int frame,int t,int c,int q,int imaginary){
 if(frame==2)return 0;
 int n=imaginary?(frame*7+t*5+c*11+q*3)%23-11:(frame*11+t*7+c*3+q*5)%29-14;
 return n/(residual?512.0f:256.0f)*32768.0f;
}
struct Paths{char*input;char*output;};
static void*run(void*arg){struct Paths*p=arg;FILE*in=fopen(p->input,"rb"),*out=fopen(p->output,"wb");if(!in||!out)exit(3);uint32_t count;if(!get(in,&count,4))exit(4);
 for(uint32_t n=0;n<count;n++){
  uint32_t tree,bands,bins,mode,boxes,cutoffs[8],modes[23],map[128];if(!get(in,&tree,4)||!get(in,&bands,4)||!get(in,&bins,4)||!get(in,&mode,4)||!get(in,&boxes,4)||tree>7||bands>23||bins>128||boxes>8)exit(5);
  if(!get(in,cutoffs,boxes*4)||!get(in,modes,bands*4)||!get(in,map,bins*4))exit(6);
  spatialDec*s=calloc(1,sizeof(*s));if(!s)exit(7);int direct=tree==0||tree==1||tree==7?1:tree==5||tree==6?6:3;int channels=tree==0||tree==1?5:tree==7?2:tree==2?6:8;int inputs=tree==0||tree==1||tree==7?1:tree==5||tree==6?6:3;int outputs=tree==7?2:tree<=2?6:8;
  s->treeConfig=tree;s->numInputChannels=inputs==3?2:inputs;s->numXChannels=inputs;s->numVChannels=channels;s->numWChannels=channels;s->numDirektSignals=direct;s->numDecorSignals=channels-direct;s->numOutputChannels=outputs;s->numParameterBands=bands;s->hybridBands=bins;s->qmfBands=bins;s->upmixType=0;
  for(uint32_t b=0;b<boxes;b++)s->resBands[b]=cutoffs[b];for(uint32_t q=0;q<bins;q++){s->kernels[q][0]=map[q];s->kernels[q][1]=1;}
  for(int j=0;j<channels-direct;j++){int seed=j;if(tree==2&&j==2)seed=3;if(tree==7)seed=2;if(SpatialDecDecorrelateCreate(&s->apDecor[j],bins,seed,mode,0,0)!=ERR_NONE)exit(8);}
  for(int frame=0;frame<3;frame++){
   uint32_t sets,slots,param[3];if(!get(in,&sets,4)||!get(in,&slots,4)||sets==0||sets>3||slots>32||!get(in,param,sets*4))exit(9);s->numParameterSets=sets;s->timeSlots=slots;
   for(uint32_t ps=0;ps<sets;ps++)s->paramSlot[ps]=param[ps];
   for(uint32_t ps=0;ps<sets;ps++)for(uint32_t pb=0;pb<bands;pb++)for(int plane=0;plane<2;plane++)for(int r=0;r<channels;r++)for(int c=0;c<inputs;c++){float v;if(!get(in,&v,4))exit(10);if(plane==0)s->M1paramReal[ps][pb][r][c]=v;else s->M1paramImag[ps][pb][r][c]=v;}
   for(int plane=0;plane<2;plane++)for(uint32_t ps=0;ps<sets;ps++)for(uint32_t pb=0;pb<bands;pb++)for(int r=0;r<outputs;r++)for(int c=0;c<channels;c++){float v;if(!get(in,&v,4))exit(11);if(plane==0)s->M2residReal[ps][pb][r][c]=v;else s->M2decorReal[ps][pb][r][c]=v;}
   for(uint32_t t=0;t<slots;t++)for(uint32_t q=0;q<bins;q++){
    for(int c=0;c<s->numInputChannels;c++){s->xReal[c][t][q]=sample(0,frame,t,c,q,0);s->xImag[c][t][q]=sample(0,frame,t,c,q,1);}
    for(uint32_t b=0;b<boxes;b++){s->hybResidualReal[b][t][q]=sample(1,frame,t,b,q,0);s->hybResidualImag[b][t][q]=sample(1,frame,t,b,q,1);}
    if(inputs==3){int tt=boxes-1;int active=map[q]<cutoffs[tt];s->xReal[2][t][q]=active?s->hybResidualReal[tt][t][q]:0;s->xImag[2][t][q]=active?s->hybResidualImag[tt][t][q]:0;}
   }
   SpatialDecApplyM1(s);SpatialDecDecorrelate(s);
   /* The configured controls have at most two contiguous mode regions. */
   int split=0;while(split<(int)bands&&modes[split]==modes[0])split++;
   s->tttConfig[0][0].stopBand=split;s->tttConfig[0][0].useTTTdecorr=(tree>=2&&tree<=4&&modes[0]==0);s->tttConfig[1][0].startBand=split;s->tttConfig[1][0].useTTTdecorr=(tree>=2&&tree<=4&&split<(int)bands&&modes[split]==0);
   SpatialDecMergeResDecor(s);SpatialDecApplyM2(s);
   for(int plane=0;plane<2;plane++)for(uint32_t t=0;t<slots;t++)for(int c=0;c<outputs;c++)for(uint32_t q=0;q<bins;q++){float re=plane?s->hybOutputRealWet[c][t][q]:s->hybOutputRealDry[c][t][q],im=plane?s->hybOutputImagWet[c][t][q]:s->hybOutputImagDry[c][t][q];float v[2]={re/32768.0f,im/32768.0f};if(!isfinite(v[0])||!isfinite(v[1])||fwrite(v,4,2,out)!=2)exit(12);}
   for(uint32_t pb=0;pb<bands;pb++)for(int r=0;r<channels;r++)for(int c=0;c<inputs;c++){s->M1paramRealPrev[pb][r][c]=s->M1paramReal[sets-1][pb][r][c];s->M1paramImagPrev[pb][r][c]=s->M1paramImag[sets-1][pb][r][c];}
   for(uint32_t pb=0;pb<bands;pb++)for(int r=0;r<outputs;r++)for(int c=0;c<channels;c++){s->M2residRealPrev[pb][r][c]=s->M2residReal[sets-1][pb][r][c];s->M2decorRealPrev[pb][r][c]=s->M2decorReal[sets-1][pb][r][c];}
  }
  for(int j=0;j<channels-direct;j++)SpatialDecDecorrelateDestroy(&s->apDecor[j]);free(s);
 }
 fclose(in);fclose(out);return NULL;
}
int main(int argc,char**argv){if(argc!=3)return 2;struct Paths p={argv[1],argv[2]};pthread_attr_t a;pthread_t t;if(pthread_attr_init(&a)||pthread_attr_setstacksize(&a,64*1024*1024)||pthread_create(&t,&a,run,&p)||pthread_join(t,NULL))return 13;pthread_attr_destroy(&a);return 0;}
