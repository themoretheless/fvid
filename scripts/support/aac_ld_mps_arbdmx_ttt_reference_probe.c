/* Research-only wrapper of the unmodified external ISO full-output routines. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "sac_calcM1andM2.c"
static int get(FILE*f,void*v,size_t n){return fread(v,1,n,f)==n;}
static void put(FILE*f,float v){double d=v;if(fwrite(&d,8,1,f)!=1)exit(8);}
int main(int argc,char**argv){
 if(argc!=3)return 2;FILE*in=fopen(argv[1],"rb"),*out=fopen(argv[2],"wb");if(!in||!out)return 3;
 spatialDec*s=calloc(1,sizeof(*s));if(!s)return 4;uint32_t count;if(!get(in,&count,4))return 5;
 for(uint32_t n=0;n<count;n++){
  uint32_t tree,mode,residual,inversion,active,boxes;double x,y,rho,gain,gain_right;
  if(!get(in,&tree,4)||!get(in,&mode,4)||!get(in,&residual,4)||!get(in,&inversion,4)||!get(in,&active,4)||!get(in,&boxes,4)||!get(in,&x,8)||!get(in,&y,8)||!get(in,&rho,8)||!get(in,&gain,8)||!get(in,&gain_right,8))return 6;
  if(tree<2||tree>4)return 11;
  int channels=(tree==0||tree==1)?5:tree==7?2:tree==2?6:8,outputs=tree==7?2:tree<=2?6:8,inputs=(tree==0||tree==1||tree==7)?1:(tree==5||tree==6)?6:3;
  s->numParameterSets=1;s->numParameterBands=1;s->arbitraryDownmix=1;s->arbdmxGain[0][0][0]=gain;s->arbdmxGain[1][0][0]=gain_right;s->_3DStereoInversion=0;s->mtxInversion=inversion;s->numOttBoxes=boxes;s->numXChannels=inputs;s->numInputChannels=inputs==3?2:inputs;
  s->tttConfig[0][0].mode=mode;s->tttConfig[0][0].startBand=0;s->tttConfig[0][0].stopBand=1;s->tttConfig[0][0].useTTTdecorr=mode==0;
  s->tttConfig[1][0].startBand=1;s->tttConfig[1][0].stopBand=1;
  s->tttCPC1[0][0][0]=x;s->tttCPC2[0][0][0]=y;s->tttCLD1[0][0][0]=x;s->tttCLD2[0][0][0]=y;s->tttICC[0][0][0]=rho;
  memset(s->M1paramReal[0][0],0,sizeof(s->M1paramReal[0][0]));memset(s->M1paramImag[0][0],0,sizeof(s->M1paramImag[0][0]));memset(s->M2residReal[0][0],0,sizeof(s->M2residReal[0][0]));memset(s->M2decorReal[0][0],0,sizeof(s->M2decorReal[0][0]));
  for(uint32_t b=0;b<boxes;b++){double cld,icc;uint32_t r;if(!get(in,&cld,8)||!get(in,&icc,8)||!get(in,&r,4))return 7;s->ottCLD[b][0][0]=cld;s->ottICC[b][0][0]=icc;s->resBands[b]=r;s->numOttBands[b]=((tree==0&&b==4)||(tree==1&&b==2)||(tree>=2&&tree<=4&&b==0))?active:1;}
  s->resBands[boxes]=residual;
  switch(tree){case 0:SpatialDecCalculateM1andM2_5151(s);break;case 1:SpatialDecCalculateM1andM2_5152(s);break;case 5:SpatialDecCalculateM1andM2_7571(s);break;case 6:SpatialDecCalculateM1andM2_7572(s);break;case 7:SpatialDecCalculateM1andM2_212(s);break;case 2:SpatialDecCalculateM1andM2_5251(s);break;case 3:SpatialDecCalculateM1andM2_7271(s);break;case 4:SpatialDecCalculateM1andM2_7272(s);break;default:return 9;}
  for(int r=0;r<channels;r++)for(int c=0;c<inputs;c++){put(out,s->M1paramReal[0][0][r][c]);put(out,s->M1paramImag[0][0][r][c]);}
  for(int r=0;r<outputs;r++)for(int c=0;c<channels;c++)put(out,s->M2residReal[0][0][r][c]);
  for(int r=0;r<outputs;r++)for(int c=0;c<channels;c++)put(out,s->M2decorReal[0][0][r][c]);
 }
 free(s);fclose(in);return fclose(out)?10:0;
}
