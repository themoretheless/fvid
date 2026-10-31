/* Explicit research driver;unmodified ISO parser and entropy sources only. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "sac_bitdec.c"
int debug[256];
void myexit(char*s){fprintf(stderr,"%s\n",s);exit(7);}
struct Reader{BYTE_READER base;const unsigned char*data;size_t at,size;};
static int next(HANDLE_BYTE_READER v){struct Reader*r=(struct Reader*)v;return r->at<r->size?r->data[r->at++]:-1;}
int main(int argc,char**argv){
 if(argc!=2)return 2;FILE*in=fopen(argv[1],"rb");if(!in)return 3;uint32_t h[4];
 spatialDec*s=calloc(1,sizeof(*s));LOSSLESSDATA*ll=calloc(1,sizeof(*ll));
 int(*data)[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS]=calloc(MAX_NUM_OTT,sizeof(*data));
 int(*last)[MAX_PARAMETER_BANDS]=calloc(MAX_NUM_OTT,sizeof(*last));
 if(!s||!ll||!data||!last)return 4;
 while(fread(h,4,4,in)==4){
  int kind=h[0],independent=h[1],stride=h[2],bytes=h[3];if(kind>2||stride<1||stride>3||bytes>1024)return 5;
  unsigned char*wire=malloc(bytes);if(fread(wire,1,bytes,in)!=(size_t)bytes)return 6;
  memset(s,0,sizeof(*s));memset(ll,0,sizeof(*ll));memset(data,0,MAX_NUM_OTT*sizeof(*data));memset(last,0,MAX_NUM_OTT*sizeof(*last));
  s->numParameterSets=2;s->bsFrame[0].bsIndependencyFlag=independent;
  struct Reader r={{next},wire,0,bytes};HANDLE_S_BITINPUT bits=s_bitinput_open(&r.base);int map[24]={0};int n=getStrideMap(stride,0,0,map);
  ecDataDec(s,bits,ll,data,last,kind,0,0,0,0,kind==0?15:kind==2?10:0);
  printf("{\"bits\":%ld,\"groups\":%d,\"map\":[%d,%d],\"dummy\":[%d,%d],\"history\":%d}\n",s_getNumBits(bits),n,map[0],map[1],data[0][0][0],data[0][1][0],last[0][0]);
  s_bitinput_close(bits);free(wire);
 }
 free(last);free(data);free(ll);free(s);fclose(in);return 0;
}
