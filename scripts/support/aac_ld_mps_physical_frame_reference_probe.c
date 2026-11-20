/* Research harness only: external ISO sources, no foreign runtime in FVid. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "physical-frame-reference.c"
int debug[256];
void myexit(char *s){fprintf(stderr,"%s\n",s);exit(7);}
BLOCKTYPE decodeIcs(spatialDec*d,int ch,int fr,int ex){myexit("residual audio outside probe scope");return 0;}
BLOCKTYPE decodeCpe(spatialDec*d,int ch,int fr,int ex){myexit("residual audio outside probe scope");return 0;}
struct Reader {BYTE_READER base;const unsigned char *p;size_t at,n;};
static int next(HANDLE_BYTE_READER p){struct Reader*r=(struct Reader*)p;return r->at<r->n?r->p[r->at++]:-1;}
static void array(int*v,int n){printf("[");for(int i=0;i<n;i++)printf("%s%d",i?",":"",v[i]);printf("]");}
static int defaults[8][5]={{15,15,0,0,15},{15,0,15,15,15},{15,15,15},{15,15,15,15,15},{15,15,15,15,15},{15,15},{15,15},{15}};
static int shape[2][8]={{5,5,4,6,6,4,4,2},{5,5,5,7,7,4,4,2}};
static void setup(spatialDec*d){
 SPATIAL_BS_CONFIG*c=&d->bsConfig;d->treeConfig=c->bsTreeConfig;d->timeSlots=c->bsFrameLength+1;d->bitstreamParameterBands=freqResTable[c->bsFreqRes];d->numParameterBands=d->bitstreamParameterBands;d->quantMode=c->bsQuantMode;d->oneIcc=c->bsOneIcc;d->tempShapeConfig=c->bsTempShapeConfig;d->arbitraryDownmix=c->bsArbitraryDownmix;d->ICCdefault=0;d->CPCdefault=10;d->tttCLD1default[0]=15;d->tttCLD2default[0]=0;
 for(int i=0;i<d->numOttBoxes;i++){d->ottModeLfe[i]=treePropertyTable[d->treeConfig].ottModeLfe[i];d->bitstreamOttBands[i]=d->ottModeLfe[i]?c->bsOttBands[i]:d->bitstreamParameterBands;d->ottCLDdefault[i]=defaults[d->treeConfig][i];}
 for(int i=0;i<d->numTttBoxes;i++){int split=c->bsTttDualMode[i]?c->bsTttBandsLow[i]:d->bitstreamParameterBands;d->tttConfig[0][i].mode=c->bsTttModeLow[i];d->tttConfig[0][i].bitstreamStartBand=0;d->tttConfig[0][i].bitstreamStopBand=split;d->tttConfig[1][i].mode=c->bsTttModeHigh[i];d->tttConfig[1][i].bitstreamStartBand=split;d->tttConfig[1][i].bitstreamStopBand=d->bitstreamParameterBands;for(int h=0;h<2;h++){d->tttConfig[h][i].startBand=d->tttConfig[h][i].bitstreamStartBand;d->tttConfig[h][i].stopBand=d->tttConfig[h][i].bitstreamStopBand;}}
}
static void region(spatialDec*d,uint32_t*h){
 SPATIAL_BS_FRAME*f=&d->bsFrame[0];int category=h[0],index=h[1],high=h[2],component=h[3],start=h[4],stop=h[5],kind=h[6],def=(int32_t)h[7],param=h[8];
 LOSSLESSDATA*ll;float(*output)[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS];int(*out)[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS],(*cmp)[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS],(*prev)[MAX_PARAMETER_BANDS];
 switch(category){
 #define PICK(L,O,V,C,P) ll=&f->L;output=d->O;out=f->V;cmp=f->C;prev=f->P;break
 case 0:PICK(CLDLosslessData,ottCLD,ottCLDidx,cmpOttCLDidx,ottCLDidxPrev);
 case 1:PICK(ICCLosslessData,ottICC,ottICCidx,cmpOttICCidx,ottICCidxPrev);
 case 2:if(component==0){ll=&f->CPCLosslessData;output=d->tttCPC1;out=f->tttCPC1idx;cmp=f->cmpTttCPC1idx;prev=f->tttCPC1idxPrev;}else{ll=&f->CPCLosslessData;output=d->tttCPC2;out=f->tttCPC2idx;cmp=f->cmpTttCPC2idx;prev=f->tttCPC2idxPrev;}break;
 case 3:if(component==0){ll=&f->CLDLosslessData;output=d->tttCLD1;out=f->tttCLD1idx;cmp=f->cmpTttCLD1idx;prev=f->tttCLD1idxPrev;}else{ll=&f->CLDLosslessData;output=d->tttCLD2;out=f->tttCLD2idx;cmp=f->cmpTttCLD2idx;prev=f->tttCLD2idxPrev;}break;
 case 4:PICK(ICCLosslessData,tttICC,tttICCidx,cmpTttICCidx,tttICCidxPrev);
 case 5:PICK(CLDLosslessData,arbdmxGain,arbdmxGainIdx,cmpArbdmxGainIdx,arbdmxGainIdxPrev);
 default:myexit("bad descriptor");return;
 }
 if(category==1&&d->oneIcc){for(index=0;index<d->numOttBoxes;index++)if(!d->ottModeLfe[index])break;}
 printf("{\"indices\":[");for(int ps=0;ps<d->numParameterSets;ps++){printf("%s",ps?",":"");array(out[index][ps]+start,stop-start);}printf("],\"values\":[");
 for(int ps=0;ps<d->numParameterSets;ps++){printf("%s[",ps?",":"");for(int b=start;b<stop;b++)printf("%s%.17g",b>start?",":"",output[index][ps][b]);printf("]");}printf("]}");
}
int main(int argc,char**argv){
 if(argc!=2)return 2;FILE*in=fopen(argv[1],"rb");if(!in)return 3;uint32_t header[2];
 while(fread(header,4,2,in)==2){
  unsigned char*cfg=malloc(header[0]);if(fread(cfg,1,header[0],in)!=header[0])return 4;
  spatialDec*d=calloc(1,sizeof(*d));struct Reader c={{next},cfg,0,header[0]};d->bitstream=s_bitinput_open(&c.base);SpatialDecParseSpecificConfig(d,header[0]);s_bitinput_close(d->bitstream);free(cfg);setup(d);
  for(unsigned fi=0;fi<header[1];fi++){
   uint32_t frame[2];if(fread(frame,4,2,in)!=2)return 5;unsigned char*p=malloc(frame[0]);if(fread(p,1,frame[0],in)!=frame[0])return 6;
   struct Reader r={{next},p,0,frame[0]};d->bitstream=s_bitinput_open(&r.base);d->parseNextBitstreamFrame=1;SpatialDecParseFrame(d);SpatialDecDecodeFrame(d);
   SPATIAL_BS_FRAME*f=&d->bsFrame[0];printf("{\"bits\":%ld,\"slots\":",s_getNumBits(d->bitstream));array(d->paramSlot,d->numParameterSets);printf(",\"independent\":%d,\"regions\":[",f->bsIndependencyFlag);
   for(unsigned i=0;i<frame[1];i++){uint32_t desc[9];if(fread(desc,4,9,in)!=9)return 8;printf("%s",i?",":"");region(d,desc);}printf("],\"smoothing\":[");
   for(int ps=0;ps<d->numParameterSets-d->extendFrame;ps++){int mode=f->bsSmoothMode[ps];printf("%s{\"mode\":%d,\"time\":",ps?",":"",mode);if(mode>=2)printf("%d",f->bsSmoothTime[ps]);else printf("null");printf(",\"stride\":");if(mode==3)printf("%d",pbStrideTable[f->bsFreqResStrideSmg[ps]]);else printf("null");printf(",\"groups\":");array(f->bsSmgData[ps],mode==3?(d->bitstreamParameterBands-1)/pbStrideTable[f->bsFreqResStrideSmg[ps]]+1:0);printf("}");}
   printf("],\"shape\":[");int count=d->tempShapeConfig?shape[d->tempShapeConfig-1][d->treeConfig]:0;
   for(int ch=0;ch<count;ch++){int enabled=d->tempShapeConfig==1?d->tempShapeEnableChannelSTP[ch]:d->tempShapeEnableChannelGES[ch];printf("%s{\"enabled\":%d,\"envelope\":[",ch?",":"",enabled);if(d->tempShapeConfig==2&&enabled)for(int ts=0;ts<d->timeSlots;ts++)printf("%s%.9g",ts?",":"",d->envShapeData[ch][ts]);printf("]}");}printf("]}\n");
   s_bitinput_close(d->bitstream);free(p);
  }
  free(d);
 }
 fclose(in);return 0;
}
