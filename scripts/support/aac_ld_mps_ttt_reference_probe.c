/* Explicit research harness; the original implementation remains external. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include "sac_calcM1andM2.c"
static int readn(FILE*f,void*p,size_t n){return fread(p,1,n,f)==n;}
static void put(FILE*f,float x){double d=x;if(fwrite(&d,8,1,f)!=1)exit(9);}
int main(int argc,char**argv){
 if(argc!=3)return 2;FILE*in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 3;
 spatialDec*s=calloc(1,sizeof(*s));if(!s)return 4;uint32_t count;if(!readn(in,&count,4))return 5;
 for(uint32_t n=0;n<count;n++){
  uint32_t mode,residual;double x,y,rho;float matrix[3][3];
  if(!readn(in,&mode,4)||!readn(in,&x,8)||!readn(in,&y,8)||!readn(in,&rho,8)||!readn(in,&residual,4))return 6;
  s->tttCPC1[0][0][0]=x;s->tttCPC2[0][0][0]=y;s->tttCLD1[0][0][0]=x;s->tttCLD2[0][0][0]=y;s->tttICC[0][0][0]=rho;s->numOttBoxes=0;s->resBands[0]=residual;
  CalculateTTT(s,0,0,mode,matrix);for(int r=0;r<3;r++)for(int c=0;c<3;c++)put(out,matrix[r][c]);
 }
 if(!readn(in,&count,4))return 7;
 for(uint32_t n=0;n<count;n++){
  uint32_t mode;double l,r,x,y;float a,b,re[2][2],im[2][2];
  if(!readn(in,&mode,4)||!readn(in,&l,8)||!readn(in,&r,8)||!readn(in,&x,8)||!readn(in,&y,8))return 8;
  getMatrixInversionWeights(l,r,mode<2,x,y,&a,&b);invertMatrix(a,b,re,im);put(out,a);put(out,b);
  for(int i=0;i<2;i++)for(int j=0;j<2;j++){put(out,re[i][j]);put(out,im[i][j]);}
 }
 free(s);fclose(in);return fclose(out)?10:0;
}
