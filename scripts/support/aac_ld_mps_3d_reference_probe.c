/* Research wrapper:unmodified ISO TTT/3D matrix routines,one selected band. */
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
  uint32_t tree,mode,residual,inversion,active,boxes,pb,hrtf[5][3];double x,y,rho,gain[2],surround;
  if(!get(in,&tree,4)||!get(in,&mode,4)||!get(in,&residual,4)||!get(in,&inversion,4)||!get(in,&active,4)||!get(in,&boxes,4)||!get(in,&x,8)||!get(in,&y,8)||!get(in,&rho,8)||!get(in,gain,16)||!get(in,&pb,4)||!get(in,&surround,8)||!get(in,hrtf,sizeof(hrtf)))return 6;
  if(tree<2||tree>4||pb>=23||boxes>5)return 9;
  double cld[5],icc[5];uint32_t res[5];for(uint32_t b=0;b<boxes;b++){if(!get(in,&cld[b],8)||!get(in,&icc[b],8)||!get(in,&res[b],4))return 7;}
  int channels=tree==2?6:8,outputs=tree==2?6:8;
  s->numParameterSets=1;s->numParameterBands=pb+1;s->arbitraryDownmix=1;s->_3DStereoInversion=1;s->mtxInversion=inversion;s->numOttBoxes=boxes;s->numXChannels=3;s->numInputChannels=2;s->surroundGain=surround;s->_3DStereoData.hrtfNumChannels=5;s->_3DStereoData.hrtfParameterBands=pb+1;
  s->tttConfig[0][0].mode=mode;s->tttConfig[0][0].startBand=0;s->tttConfig[0][0].stopBand=pb+1;s->tttConfig[0][0].useTTTdecorr=mode==0;s->tttConfig[1][0].startBand=pb+1;s->tttConfig[1][0].stopBand=pb+1;
  for(uint32_t p=0;p<=pb;p++){
   s->tttCPC1[0][0][p]=x;s->tttCPC2[0][0][p]=y;s->tttCLD1[0][0][p]=x;s->tttCLD2[0][0][p]=y;s->tttICC[0][0][p]=rho;s->arbdmxGain[0][0][p]=gain[0];s->arbdmxGain[1][0][p]=gain[1];
   memset(s->M1paramReal[0][p],0,sizeof(s->M1paramReal[0][p]));memset(s->M1paramImag[0][p],0,sizeof(s->M1paramImag[0][p]));memset(s->M2residReal[0][p],0,sizeof(s->M2residReal[0][p]));memset(s->M2decorReal[0][p],0,sizeof(s->M2decorReal[0][p]));
   for(uint32_t b=0;b<boxes;b++){s->ottCLD[b][0][p]=cld[b];s->ottICC[b][0][p]=icc[b];s->resBands[b]=res[b];s->numOttBands[b]=b==0?(active?pb+1:0):pb+1;}
   for(int ch=0;ch<5;ch++){HRTF_DIRECTIONAL_DATA*h=&s->_3DStereoData.directional[ch];h->powerL[p]=(float)pow(10.f,((int)hrtf[ch][0]-16)/20.f);h->powerR[p]=(float)pow(10.f,((int)hrtf[ch][1]-16)/20.f);h->ipd[p]=hrtf[ch][2]*((float)C_PI/16.f);h->icc[p]=1.f;}
  }
  s->resBands[boxes]=residual;
  switch(tree){case 2:SpatialDecCalculateM1andM2_5251(s);break;case 3:SpatialDecCalculateM1andM2_7271(s);break;case 4:SpatialDecCalculateM1andM2_7272(s);break;}
  for(int r=0;r<channels;r++)for(int c=0;c<3;c++){put(out,s->M1paramReal[0][pb][r][c]);put(out,s->M1paramImag[0][pb][r][c]);}
  for(int r=0;r<outputs;r++)for(int c=0;c<channels;c++)put(out,s->M2residReal[0][pb][r][c]);
  for(int r=0;r<outputs;r++)for(int c=0;c<channels;c++)put(out,s->M2decorReal[0][pb][r][c]);
  /* Independent polar oracle receives the original un-inverted C input basis.
     Keep stock3D outputs above as a separately measurable numerical control. */
  s->_3DStereoInversion=0;
  switch(tree){case 2:SpatialDecCalculateM1andM2_5251(s);break;case 3:SpatialDecCalculateM1andM2_7271(s);break;case 4:SpatialDecCalculateM1andM2_7272(s);break;}
  for(int r=0;r<channels;r++)for(int c=0;c<3;c++){put(out,s->M1paramReal[0][pb][r][c]);put(out,s->M1paramImag[0][pb][r][c]);}
  float pre[3][3],g[2];CalculateTTT(s,0,pb,mode,pre);CalculateArbDmxMtx(s,0,pb,g);
  for(int r=0;r<3;r++)for(int c=0;c<3;c++)put(out,pre[r][c]*(c<2?g[c]:1.f));
 }
 free(s);fclose(in);return fclose(out)?10:0;
}
