#include <stdio.h>
#include <stdlib.h>
#include "sac_polyphase.h"
int main(int argc,char **argv) {
 if(argc!=5 && argc!=6)return 2;int bands=atoi(argv[1]);
 FILE *in=fopen(argv[2],"rb"),*analysis=fopen(argv[3],"wb"),*synthesis=fopen(argv[4],"wb");
 if(!in||!analysis||!synthesis)return 3;
 SAC_POLYPHASE_ANA_FILTERBANK *a;SAC_POLYPHASE_SYN_FILTERBANK *s;
 SacInitAnaFilterbank(NULL,bands);SacInitSynFilterbank(NULL,bands);SacOpenAnaFilterbank(&a);SacOpenSynFilterbank(&s);
 float pcm[64],re[64],im[64];double out[64];
 if(argc==6) {
  float packed[128];
  while(fread(packed,sizeof(float),2*bands,in)==2*bands){
   for(int k=0;k<bands;k++){re[k]=packed[2*k];im[k]=packed[2*k+1];}
   SacCalculateSynFilterbank(s,re,im,out);fwrite(out,8,bands,synthesis);
  }
 } else while(fread(pcm,sizeof(float),bands,in)==bands){
  SacCalculateAnaFilterbank(a,pcm,re,im);
  for(int k=0;k<bands;k++){fwrite(&re[k],4,1,analysis);fwrite(&im[k],4,1,analysis);}
  SacCalculateSynFilterbank(s,re,im,out);fwrite(out,8,bands,synthesis);
 }
 SacCloseAnaFilterbank(a);SacCloseSynFilterbank(s);fclose(in);fclose(analysis);fclose(synthesis);return 0;
}
