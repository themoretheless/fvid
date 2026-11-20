/* Explicit research wrapper around the external, unchanged ISO interpolator. */
#include "sac_calcM1andM2.c"
void probe_interp(spatialDec *d,
 float m[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT],
 float out[MAX_TIME_SLOTS][MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT],
 float previous[MAX_PARAMETER_BANDS][MAX_M_OUTPUT][MAX_M_INPUT],int rows,int cols){
 interp_UMX(m,out,previous,rows,cols,d);
}
