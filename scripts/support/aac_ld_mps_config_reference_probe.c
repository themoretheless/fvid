#include <stdio.h>
#include <stdlib.h>
#include "sac_bitdec.h"
#include "sac_bitinput.h"
int debug[256];
void myexit(char *message){fprintf(stderr,"%s\n",message);exit(7);}
struct Reader {BYTE_READER base;FILE *file;};
static int next(HANDLE_BYTE_READER r){return fgetc(((struct Reader*)r)->file);}
static void array(const int *v,int n){printf("[");for(int i=0;i<n;i++)printf("%s%d",i?",":"",v[i]);printf("]");}
int main(int argc,char **argv){
 if(argc!=2)return 2;FILE *f=fopen(argv[1],"rb");if(!f)return 3;fseek(f,0,SEEK_END);long len=ftell(f);rewind(f);
 struct Reader reader={{next},f};spatialDec *d=calloc(1,sizeof(*d));d->bitstream=s_bitinput_open(&reader.base);
 SpatialDecParseSpecificConfig(d,(int)len);SPATIAL_BS_CONFIG *c=&d->bsConfig;
 printf("{\"head\":[%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d,%d],\"ott\":",c->bsSamplingFreqIndex,c->bsSamplingFrequency,c->bsFrameLength,c->bsFreqRes,c->bsTreeConfig,c->bsQuantMode,c->bsOneIcc,c->bsArbitraryDownmix,c->bsFixedGainSur,c->bsFixedGainLFE,c->bsFixedGainDMX,c->bsMatrixMode,c->bsTempShapeConfig,c->bsDecorrConfig,c->bs3DaudioMode,c->bs3DaudioHRTFset,c->bsHRTFfreqRes,c->bsHRTFasymmetric,c->bsEnvQuantMode,d->numInputChannels,d->numOutputChannels,d->numOttBoxes,d->numTttBoxes);
 array(c->bsOttBands,d->numOttBoxes);printf(",\"ttt\":[");for(int i=0;i<d->numTttBoxes;i++)printf("%s[%d,%d,%d,%d]",i?",":"",c->bsTttDualMode[i],c->bsTttModeLow[i],c->bsTttModeHigh[i],c->bsTttBandsLow[i]);printf("],\"hrtf\":[");for(int i=0;i<c->bsHRTFnumChan;i++){printf("%s[",i?",":"");array(c->bsHRTFlevelLeft[i],c->HRTFnumBand);printf(",");array(c->bsHRTFlevelRight[i],c->HRTFnumBand);printf(",%d,",c->bsHRTFphase[i]);array(c->bsHRTFphaseLR[i],c->HRTFnumBand);printf("]");}printf("],\"extensions\":");array(c->bsSacExtType,c->sacExtCnt);
 printf(",\"residual\":[%d,%d,",c->bsResidualSamplingFreqIndex,c->bsResidualFramesPerSpatialFrame);array(c->bsResidualPresent,d->numOttBoxes+d->numTttBoxes);printf(",");array(c->bsResidualBands,d->numOttBoxes+d->numTttBoxes);printf("],\"dmx_residual\":[%d,%d,%d],\"arbitrary\":[%d,%d,",c->bsArbitraryDownmixResidualSamplingFreqIndex,c->bsArbitraryDownmixResidualFramesPerSpatialFrame,c->bsArbitraryDownmixResidualBands,c->numOttBoxesAT,c->numOutChanAT);array(c->bsOutputChannelPosAT,c->numOutChanAT);printf(",");array(c->bsOttDefaultCldAT,c->numOttBoxesAT);printf(",");array(c->bsOttModeLfeAT,c->numOttBoxesAT);printf(",");array(c->bsOttBandsAT,c->numOttBoxesAT);printf("]}\n");
 s_bitinput_close(d->bitstream);free(d);fclose(f);return 0;
}
