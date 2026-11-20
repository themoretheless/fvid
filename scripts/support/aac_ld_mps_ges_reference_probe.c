/* Explicit reference-only harness. No part of this probe enters the library. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "sac_reshapeBBEnv.c"
static uint32_t word(FILE*f){uint32_t v;if(fread(&v,4,1,f)!=1)exit(3);return v;}
static float sample(FILE*f){float v;if(fread(&v,4,1,f)!=1)exit(4);return v;}
int main(int argc,char**argv){
 if(argc!=3)return 2;FILE*in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 5;
 uint32_t count=word(in);for(uint32_t p=0;p<count;p++){
  spatialDec*s=calloc(1,sizeof(*s));if(!s)return 6;
  s->treeConfig=word(in);s->hybridBands=word(in);s->numInputChannels=word(in);s->numOutputChannels=word(in);int blocks=word(in);s->numParameterBands=23;s->tempShapeConfig=2;SacInitBBEnv(s);
  for(int block=0;block<blocks;block++){
   s->timeSlots=word(in);int channels=word(in);
   for(int ch=0;ch<channels;ch++){s->tempShapeEnableChannelGES[ch]=word(in);if(s->tempShapeEnableChannelGES[ch])for(int t=0;t<s->timeSlots;t++)s->envShapeData[ch][t]=sample(in);}
   for(int kind=0;kind<3;kind++)for(int t=0;t<s->timeSlots;t++)for(int ch=0;ch<(kind?s->numOutputChannels:s->numInputChannels);ch++)for(int b=0;b<s->hybridBands;b++){
    float re=sample(in)*32768.f,im=sample(in)*32768.f;
    if(kind==0){s->hybInputReal[ch][t][b]=re;s->hybInputImag[ch][t][b]=im;}
    if(kind==1){s->hybOutputRealDry[ch][t][b]=re;s->hybOutputImagDry[ch][t][b]=im;}
    if(kind==2){s->hybOutputRealWet[ch][t][b]=re;s->hybOutputImagWet[ch][t][b]=im;}
   }
   spatialDecReshapeBBEnv(s);
   for(int t=0;t<s->timeSlots;t++)for(int ch=0;ch<s->numOutputChannels;ch++)for(int b=0;b<s->hybridBands;b++){
    float v[2]={s->hybOutputRealDry[ch][t][b]/32768.f,s->hybOutputImagDry[ch][t][b]/32768.f};if(fwrite(v,4,2,out)!=2)return 7;
   }
  }
  free(s);
 }
 fclose(in);return fclose(out)?8:0;
}
