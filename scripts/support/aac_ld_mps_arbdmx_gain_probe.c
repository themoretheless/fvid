/* Reference-only component wrapper; no original source is vendored. */
#include <stdio.h>
#include <stdlib.h>
#include "sac_calcM1andM2.c"
int main(void) {
    const float db[7]={-16,-6,-2,0,2,6,16};
    spatialDec *s=calloc(1,sizeof(*s)); if(!s)return 2;
    for(int inputs=1;inputs<=6;inputs+=5)for(int phase=0;phase<7;phase++) {
        s->numInputChannels=inputs;s->arbdmxResidualBands=0;
        for(int ch=0;ch<inputs;ch++)s->arbdmxGain[ch][0][0]=db[(phase+ch)%7];
        float gain[6]={0};CalculateArbDmxMtx(s,0,0,gain);
        for(int ch=0;ch<inputs;ch++)printf("%d %d %d %.17g %.17g\n",inputs,phase,ch,db[(phase+ch)%7],gain[ch]);
    }
    free(s);return 0;
}
