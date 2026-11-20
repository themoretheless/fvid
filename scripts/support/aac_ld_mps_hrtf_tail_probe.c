/* Own diagnostic wrapper; the included original ISO implementation is external. */
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#include "sac_calcM1andM2.c"
int main(void) {
  const int grids[] = {4,5,7,9,12,15,23};
  for (int s=0;s<7;s++) for (int h=0;h<7;h++) for (int tail=0;tail<2;tail++) {
    spatialDec *d=calloc(1,sizeof(*d)); if(!d)return 2;
    int pb=grids[s]-1;
    d->_3DStereoData.hrtfNumChannels=5;
    d->_3DStereoData.hrtfParameterBands=grids[h];
    d->surroundGain=1.f;
    d->ottCLD[1][0][pb]=3.f; d->ottCLD[2][0][pb]=-3.f;
    for(int c=0;c<5;c++)for(int b=0;b<23;b++) {
      HRTF_DIRECTIONAL_DATA *r=&d->_3DStereoData.directional[c];
      /* The two contexts agree on every signalled parameter. Only the
         untransmitted tail differs, with finite nonzero values in both. */
      float extra=b<grids[h]?0.f:(float)tail*0.37f;
      r->powerL[b]=0.8f+c*0.13f+b*0.01f+extra;
      r->powerR[b]=1.3f-c*0.09f+b*0.02f+extra*0.7f;
      r->ipd[b]=c*0.17f+b*0.03f; r->icc[b]=1.f;
    }
    float pre[3][5]={{1.f,0.f,0.f,0.f,0.f},{0.f,1.f,0.f,0.f,0.f},{0.3f,0.7f,0.f,0.f,0.f}};
    float re[2][2],im[2][2]; Calculate3Dinv(d,0,pb,pre,re,im);
    printf("%d %d %d",grids[s],grids[h],tail);
    for(int r=0;r<2;r++)for(int c=0;c<2;c++)printf(" %.9g %.9g",re[r][c],im[r][c]);
    putchar('\n'); free(d);
  }
  return 0;
}
