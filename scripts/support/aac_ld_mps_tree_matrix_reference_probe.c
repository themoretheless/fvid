/* Research harness only; original ISO implementation stays external. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "sac_calcM1andM2.c"
static int get(FILE *f,void *v,size_t n){return fread(v,1,n,f)==n;}
static void put(FILE *f,float v){double d=v;if(fwrite(&d,8,1,f)!=1)exit(8);}
int main(int argc,char **argv){
 if(argc!=3)return 2;FILE *in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 3;
 spatialDec *s=calloc(1,sizeof(*s));if(!s)return 4;uint32_t count;if(!get(in,&count,4))return 5;
 for(uint32_t n=0;n<count;n++){
  uint32_t tree,active,boxes;if(!get(in,&tree,4)||!get(in,&active,4)||!get(in,&boxes,4))return 6;
  int inputs=(tree==5||tree==6)?6:1,outputs=tree==7?2:inputs==6?8:6,columns=inputs==6?8:tree==7?2:5;
  s->numParameterSets=1;s->numParameterBands=1;s->arbitraryDownmix=0;
  memset(s->M1paramReal[0][0],0,sizeof(s->M1paramReal[0][0]));memset(s->M2residReal[0][0],0,sizeof(s->M2residReal[0][0]));memset(s->M2decorReal[0][0],0,sizeof(s->M2decorReal[0][0]));
  for(uint32_t b=0;b<boxes;b++){double cld,icc;uint32_t residual;if(!get(in,&cld,8)||!get(in,&icc,8)||!get(in,&residual,4))return 7;
   s->ottCLD[b][0][0]=(float)cld;s->ottICC[b][0][0]=(float)icc;s->resBands[b]=residual;s->numOttBands[b]=1;
  }
  if(tree==0)s->numOttBands[4]=active;if(tree==1)s->numOttBands[2]=active;
  switch(tree){case 0:SpatialDecCalculateM1andM2_5151(s);break;case 1:SpatialDecCalculateM1andM2_5152(s);break;case 5:SpatialDecCalculateM1andM2_7571(s);break;case 6:SpatialDecCalculateM1andM2_7572(s);break;case 7:SpatialDecCalculateM1andM2_212(s);break;default:return 9;}
  for(int r=0;r<columns;r++)for(int c=0;c<inputs;c++)put(out,s->M1paramReal[0][0][r][c]);
  for(int r=0;r<outputs;r++)for(int c=0;c<columns;c++)put(out,s->M2residReal[0][0][r][c]);
  for(int r=0;r<outputs;r++)for(int c=0;c<columns;c++)put(out,s->M2decorReal[0][0][r][c]);
 }
 free(s);fclose(in);if(fclose(out))return 10;return 0;
}
