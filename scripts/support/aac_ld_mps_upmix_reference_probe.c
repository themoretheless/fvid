/* Research-only full normalized complex-QMF comparison with original ISO DSP. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#include "sac_decor.h"
extern void probe_ott(double,double,int,double*);
int debug[256];
void myexit(char*s){fprintf(stderr,"%s\n",s);exit(7);}
static float real_in[MAX_TIME_SLOTS][MAX_HYBRID_BANDS],imag_in[MAX_TIME_SLOTS][MAX_HYBRID_BANDS],real_out[MAX_TIME_SLOTS][MAX_HYBRID_BANDS],imag_out[MAX_TIME_SLOTS][MAX_HYBRID_BANDS];
int main(int argc,char**argv){
 if(argc!=3)return 2;FILE*in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 3;uint32_t n;
 if(fread(&n,4,1,in)!=1)return 4;
 for(unsigned i=0;i<n;i++){double values[2],result[8];uint32_t residual;if(fread(values,8,2,in)!=2||fread(&residual,4,1,in)!=1)return 5;probe_ott(values[0],values[1],residual,result);fwrite(result,8,8,out);}
 if(fread(&n,4,1,in)!=1)return 6;
 for(unsigned i=0;i<n;i++){
  uint32_t h[5];if(fread(h,4,5,in)!=5)return 7;int bands=h[0],seed=h[1],config=h[2],mode=h[3],slots=h[4];HANDLE_DECORR_DEC d=NULL;
  if(SpatialDecDecorrelateCreate(&d,bands,seed,mode,0,config)!=ERR_NONE)return 8;
  for(int at=0;at<slots;at+=32){int count=slots-at<32?slots-at:32;
   for(int t=0;t<count;t++)for(int b=0;b<bands;b++){double x[2];if(fread(x,8,2,in)!=2)return 9;real_in[t][b]=(float)(x[0]*32768.0);imag_in[t][b]=(float)(x[1]*32768.0);}
   SpatialDecDecorrelateApply(d,real_in,imag_in,real_out,imag_out,count);
   for(int t=0;t<count;t++)for(int b=0;b<bands;b++){double x[2]={real_out[t][b]/32768.0,imag_out[t][b]/32768.0};if(!isfinite(x[0])||!isfinite(x[1]))return 10;fwrite(x,8,2,out);}
  }
  SpatialDecDecorrelateDestroy(&d);
 }
 fclose(in);fclose(out);return 0;
}
