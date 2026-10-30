/* Research harness only. ISO sources are supplied externally and unchanged. */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include "sac_nlc_dec.h"
int debug[256];
void myexit(char *message){fprintf(stderr,"%s\n",message);exit(7);}
struct Reader{BYTE_READER base;const unsigned char *data;size_t at,size;};
static int next(HANDLE_BYTE_READER r){struct Reader *v=(struct Reader*)r;return v->at<v->size?v->data[v->at++]:-1;}
int main(int argc,char **argv){
 if(argc!=2)return 2;FILE *in=fopen(argv[1],"rb");if(!in)return 3;uint32_t h[8];
 while(fread(h,sizeof(uint32_t),8,in)==8){
  int kind=h[0],coarse=h[1],pair=h[2],independent=h[3],set=h[4],n=h[5],bytes=h[6],start=h[7];
  if(n<0||n>23||start+n>23||set+pair>1)return 4;
  int history[MAXBANDS]={0},out[2][MAXBANDS]={{0}};int32_t input[23];
  if(fread(input,4,n,in)!=(size_t)n)return 5;
  for(int i=0;i<n;i++)history[start+i]=input[i];
  unsigned char *data=malloc(bytes);if(fread(data,1,bytes,in)!=(size_t)bytes)return 6;
  struct Reader reader={{next},data,0,bytes};HANDLE_S_BITINPUT bits=s_bitinput_open(&reader.base);
  int success=EcDataPairDec(bits,out,history,(DATA_TYPE)kind,set,start,n,pair,coarse,independent);
  printf("{\"success\":%d,\"bits\":%ld,\"values\":[",success,s_getNumBits(bits));
  for(int r=0;r<=pair;r++){printf("%s[",r?",":"");for(int k=0;k<n;k++)printf("%s%d",k?",":"",out[set+r][start+k]);printf("]");}
  printf("],\"history\":[");for(int k=0;k<n;k++)printf("%s%d",k?",":"",history[start+k]);printf("]}\n");
  s_bitinput_close(bits);free(data);
 }
 fclose(in);return 0;
}
