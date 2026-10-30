/* Research-only comparison; external ISO sources never enter production. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "physical-reference.c"
int debug[256];
void myexit(char*s){fprintf(stderr,"%s\n",s);exit(7);}
void spatialMeasureTonality(spatialDec*d,float*t){myexit("tonality outside explicit smoothing scope");}
#ifdef PROBE_DOUBLE
typedef double Real;
#else
typedef float Real;
#endif
static void number(double x){if(isfinite(x))printf("%.17g",x);else printf("null");}
static void array(int*p,int n){printf("[");for(int i=0;i<n;i++)printf("%s%d",i?",":"",p[i]);printf("]");}
typedef float MatrixSets[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT];
typedef float MatrixPrev[MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT];
extern void SpatialDecSmoothM1andM2(spatialDec*);
extern void probe_interp(spatialDec*,float[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT],float[MAX_TIME_SLOTS][MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT],float[MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT],int,int);
static float interpolated[7][MAX_TIME_SLOTS][MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT];
int main(int argc,char**argv){
 if(argc!=2)return 2;FILE*f=fopen(argv[1],"rb");if(!f)return 3;spatialDec*d=calloc(1,sizeof(*d));uint32_t type;
 MatrixSets*planes[7]={&d->M0arbdmx,&d->M1paramReal,&d->M1paramImag,&d->M2decorReal,&d->M2decorImag,&d->M2residReal,&d->M2residImag};
 MatrixPrev*previous[7]={&d->M0arbdmxPrev,&d->M1paramRealPrev,&d->M1paramImagPrev,&d->M2decorRealPrev,&d->M2decorImagPrev,&d->M2residRealPrev,&d->M2residImagPrev};
 while(fread(&type,4,1,f)==1){
  if(type==1){int32_t fields[2];double db;if(fread(fields,4,2,f)!=2||fread(&db,8,1,f)!=1)return 4;int index=fields[0];Real left=0,right=0;factorCLD(&index,(Real)db,&left,&right,fields[1]);printf("{\"type\":1,\"index\":%d,\"left\":",index);number(left);printf(",\"right\":");number(right);printf("}\n");}
  else if(type==2){uint32_t h[4];if(fread(h,4,4,f)!=4)return 5;int bands=h[0],sets=h[1],extend=h[2],reset=h[3];if(reset){d->smoothState.prevSmgTime=0;memset(d->smoothState.prevSmgData,0,sizeof(d->smoothState.prevSmgData));}d->bitstreamParameterBands=bands;d->numParameterSets=sets;d->extendFrame=extend;d->upmixType=0;
   for(int ps=0;ps<sets;ps++){uint32_t s[4];if(fread(s,4,4,f)!=4)return 6;d->bsFrame[0].bsSmoothMode[ps]=s[0];d->bsFrame[0].bsSmoothTime[ps]=s[1];d->bsFrame[0].bsFreqResStrideSmg[ps]=s[2];for(unsigned b=0;b<s[3];b++){uint32_t bit;if(fread(&bit,4,1,f)!=1)return 7;d->bsFrame[0].bsSmgData[ps][b]=bit;}}
   d->bsFrame[0].bsSmoothControl=1;decodeAndMapFrameSmg(d);printf("{\"type\":2,\"controls\":[");for(int ps=0;ps<sets+extend;ps++){printf("%s{\"time\":%d,\"mask\":",ps?",":"",d->smgTime[ps]);array(d->smgData[ps],bands);printf("}");}printf("]}\n");
  }
  else if(type==3){uint32_t h[8];if(fread(h,4,8,f)!=8)return 8;int bands=h[0],count=h[1],rows=h[2],cols=h[3],sets=h[4],time=h[5],cutoff=h[6],reset=h[7];if(reset)for(int k=0;k<7;k++)memset(previous[k],0,sizeof(MatrixPrev));d->numParameterBands=bands;d->numParameterSets=sets;d->timeSlots=time;d->smoothControl=1;d->smoothConfig=0;d->residualCoding=cutoff>0;memset(d->resBands,0,sizeof(d->resBands));d->resBands[0]=cutoff;d->arbitraryDownmix=0;
   for(int ps=0;ps<sets;ps++){uint32_t head[2];if(fread(head,4,2,f)!=2)return 9;d->paramSlot[ps]=head[0];d->smgTime[ps]=head[1];for(int b=0;b<bands;b++){uint32_t bit;if(fread(&bit,4,1,f)!=1)return 10;d->smgData[ps][b]=bit;}for(int b=0;b<bands;b++)for(int k=0;k<count;k++)for(int row=0;row<rows;row++)for(int col=0;col<cols;col++){double value;if(fread(&value,8,1,f)!=1)return 11;(*planes[k])[ps][b][row][col]=(float)value;}}
   SpatialDecSmoothM1andM2(d);for(int k=0;k<count;k++)probe_interp(d,*planes[k],interpolated[k],*previous[k],rows,cols);
   printf("{\"type\":3,\"endpoints\":[");for(int ps=0;ps<sets;ps++){printf("%s[",ps?",":"");int at=0;for(int b=0;b<bands;b++)for(int k=0;k<count;k++)for(int row=0;row<rows;row++)for(int col=0;col<cols;col++){printf("%s",at++?",":"");number((*planes[k])[ps][b][row][col]);}printf("]");}printf("],\"output\":[");
   for(int ts=0;ts<time;ts++){printf("%s[",ts?",":"");int at=0;for(int b=0;b<bands;b++)for(int k=0;k<count;k++)for(int row=0;row<rows;row++)for(int col=0;col<cols;col++){printf("%s",at++?",":"");number(interpolated[k][ts][b][row][col]);}printf("]");}printf("]}\n");
   for(int k=0;k<count;k++)for(int b=0;b<bands;b++)for(int row=0;row<rows;row++)for(int col=0;col<cols;col++)(*previous[k])[b][row][col]=(*planes[k])[sets-1][b][row][col];
  }else return 12;
 }
 free(d);fclose(f);return 0;
}
