/* Research harness; unmodified original merge and residual-index functions. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include "sac_process.c"
static int readn(FILE*f,void*p,size_t n){return fread(p,1,n,f)==n;}
static void sample(int kind,int t,int c,int q,float*re,float*im){
 if(kind==0){*re=((t*11+c*7+q*3)%31-15)/16.0f;*im=((t*5+c*13+q*7)%29-14)/16.0f;}
 else if(kind==1){*re=((t*17+c*11+q*5)%37-18)/32.0f;*im=((t*13+c*3+q*17)%43-21)/32.0f;}
 else{*re=((t*7+c*19+q*11)%41-20)/64.0f;*im=((t*3+c*17+q*13)%47-23)/64.0f;}
}
int main(int argc,char**argv){
 if(argc!=3)return 2;FILE*in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 3;
 spatialDec*s=calloc(1,sizeof(*s));if(!s)return 4;uint32_t count;if(!readn(in,&count,4))return 5;
 const int directs[8]={1,1,3,3,3,6,6,1},channels[8]={5,5,6,8,8,8,8,2};
 for(uint32_t n=0;n<count;n++){
  uint32_t tree,bands,bins,boxes,cutoffs[8],modes[23],map[128];
  if(!readn(in,&tree,4)||!readn(in,&bands,4)||!readn(in,&bins,4)||!readn(in,&boxes,4)||tree>7||bands>23||bins>128||boxes>8)return 6;
  if(!readn(in,cutoffs,boxes*4)||!readn(in,modes,bands*4)||!readn(in,map,bins*4))return 7;
  s->treeConfig=tree;s->numDirektSignals=directs[tree];s->numWChannels=channels[tree];s->timeSlots=2;s->hybridBands=1;
  for(uint32_t b=0;b<boxes;b++)s->resBands[b]=cutoffs[b];
  double result[2][2][8][128][2]={0};
  for(uint32_t q=0;q<bins;q++){
   s->kernels[0][0]=map[q];s->tttConfig[0][0].stopBand=bands;s->tttConfig[0][0].useTTTdecorr=(tree>=2&&tree<=4&&modes[map[q]]==0);s->tttConfig[1][0].startBand=bands;s->tttConfig[1][0].useTTTdecorr=0;
   for(int t=0;t<2;t++){
    for(int c=0;c<channels[tree];c++){sample(0,t,c,q,&s->vReal[c][t][0],&s->vImag[c][t][0]);sample(1,t,c,q,&s->wWetReal[c][t][0],&s->wWetImag[c][t][0]);}
    for(uint32_t b=0;b<boxes;b++)sample(2,t,b,q,&s->hybResidualReal[b][t][0],&s->hybResidualImag[b][t][0]);
   }
   SpatialDecMergeResDecor(s);
   for(int t=0;t<2;t++)for(int c=0;c<channels[tree];c++){
    result[0][t][c][q][0]=s->wDryReal[c][t][0];result[0][t][c][q][1]=s->wDryImag[c][t][0];result[1][t][c][q][0]=s->wWetReal[c][t][0];result[1][t][c][q][1]=s->wWetImag[c][t][0];
   }
  }
  for(int p=0;p<2;p++)for(int t=0;t<2;t++)for(int c=0;c<channels[tree];c++)for(uint32_t q=0;q<bins;q++)if(fwrite(result[p][t][c][q],8,2,out)!=2)return 8;
 }
 free(s);fclose(in);return fclose(out)?9:0;
}
