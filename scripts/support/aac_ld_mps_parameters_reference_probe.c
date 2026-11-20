/* Research-only harness; external ISO translation unit is never vendored. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "parameter-reference.c"
int debug[256];
void myexit(char *s){fprintf(stderr,"%s\n",s);exit(7);}
struct Reader { BYTE_READER base; const unsigned char *bytes; size_t at, size; };
static int next(HANDLE_BYTE_READER r){struct Reader *p=(struct Reader*)r;return p->at<p->size?p->bytes[p->at++]:-1;}
static int data[MAX_NUM_OTT][MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS];
static int decoded[MAX_NUM_OTT][MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS];
static int parsed[MAX_NUM_OTT][MAX_PARAMETER_BANDS], mapped[MAX_NUM_OTT][MAX_PARAMETER_BANDS];
static float output[MAX_NUM_OTT][MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS];
int main(int argc,char **argv){
 if(argc!=2)return 2;FILE *f=fopen(argv[1],"rb");if(!f)return 3;
 spatialDec *self=calloc(1,sizeof(*self));LOSSLESSDATA ll={0};uint32_t h[8];
 while(fread(h,4,8,f)==8){
  int kind=h[0],bands=h[1],def=(int32_t)h[2],sets=h[3],independent=h[4],bytes=h[5],reset=h[6],start=h[7];
  if(bands<1||bands+start>23||sets<1||sets>2||bytes>4096)return 4;
  if(reset){memset(&ll,0,sizeof(ll));memset(parsed,0,sizeof(parsed));memset(mapped,0,sizeof(mapped));for(int b=0;b<bands;b++)parsed[0][start+b]=mapped[0][start+b]=def;}
  unsigned char packet[4096];if(fread(packet,1,bytes,f)!=(size_t)bytes)return 5;
  self->numParameterSets=sets;self->curFrameBs=0;self->bsFrame[0].bsIndependencyFlag=independent;
  struct Reader r={{next},packet,0,bytes};HANDLE_S_BITINPUT bits=s_bitinput_open(&r.base);
  ecDataDec(self,bits,&ll,data,parsed,kind,0,0,start,start+bands,def);
  int slots[2]={sets==2?7:15,15};
  mapIndexData(&ll,output,decoded,data,NULL,0,mapped,0,kind,start,start+bands,def,sets,slots,0,0,NULL,NULL,NULL);
  printf("{\"bits\":%ld,\"rows\":[",s_getNumBits(bits));
  for(int ps=0;ps<sets;ps++){printf("%s[",ps?",":"");for(int b=0;b<bands;b++)printf("%s%d",b?",":"",decoded[0][ps][start+b]);printf("]");}
  printf("],\"parsed\":[");for(int b=0;b<bands;b++)printf("%s%d",b?",":"",parsed[0][start+b]);printf("]}\n");s_bitinput_close(bits);
 }
 free(self);fclose(f);return 0;
}
