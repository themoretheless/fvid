/* Reference-only harness; library/tests never invoke this executable. */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include "sac_dec.h"
void arbitraryTreeProcess(spatialDec*);
static uint32_t word(FILE*f){uint32_t v;if(fread(&v,4,1,f)!=1)exit(3);return v;}
static float sample(FILE*f){float v;if(fread(&v,4,1,f)!=1)exit(4);return v;}
int main(int argc,char**argv){
 if(argc!=3)return 2;FILE*in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 5;
 uint32_t count=word(in);for(uint32_t p=0;p<count;p++){
  spatialDec*s=calloc(1,sizeof(*s));if(!s)return 6;
  s->treeConfig=word(in);s->hybridBands=word(in);s->numOutputChannels=word(in);s->numOutputChannelsAT=word(in);s->numParameterBands=word(in);int boxes=word(in),blocks=word(in);
  s->bsConfig.arbitraryTree=1;s->bsConfig.numOttBoxesAT=boxes;
  for(int c=0;c<s->numOutputChannels;c++){int n=word(in);for(int i=0;i<n;i++)s->bsConfig.bsOttBoxPresentAT[c][i]=word(in);}
  for(int c=0;c<s->numOutputChannelsAT;c++)s->bsConfig.bsOutputChannelPosAT[c]=word(in);
  for(int b=0;b<boxes;b++)s->bsConfig.bsOttBandsAT[b]=word(in);
  for(int q=0;q<s->hybridBands;q++)s->kernels[q][0]=word(in);
  s->surroundGain=sample(in);s->lfeGain=sample(in);
  for(int c=0;c<MAX_OUTPUT_CHANNELS_AT;c++)for(int b=0;b<MAX_PARAMETER_BANDS;b++)s->prevGainAT[c][b]=1.0f;
  for(int block=0;block<blocks;block++){
   for(int c=0;c<s->numOutputChannelsAT;c++)s->bsConfig.bsOutputChannelPosAT[c]=word(in);
   s->timeSlots=word(in);s->numParameterSets=word(in);
   for(int i=0;i<s->numParameterSets;i++)s->paramSlot[i]=word(in);
   for(int i=0;i<s->numParameterSets;i++)for(int b=0;b<boxes;b++)for(int q=0;q<s->numParameterBands;q++)s->ottCLD[b][i][q]=sample(in);
   for(int t=0;t<s->timeSlots;t++)for(int c=0;c<s->numOutputChannels;c++)for(int q=0;q<s->hybridBands;q++){s->hybOutputRealDry[c][t][q]=sample(in);s->hybOutputImagDry[c][t][q]=sample(in);}
   arbitraryTreeProcess(s);
   for(int t=0;t<s->timeSlots;t++)for(int c=0;c<s->numOutputChannelsAT;c++)for(int q=0;q<s->hybridBands;q++){float z[2]={s->hybOutputRealDry[c][t][q],s->hybOutputImagDry[c][t][q]};if(fwrite(z,4,2,out)!=2)return 7;}
  }
  free(s);
 }
 fclose(in);return fclose(out)?8:0;
}
