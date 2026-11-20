/* Research wrapper; external ISO matrix implementation is never vendored. */
#include "sac_calcM1andM2.c"
void probe_ott(double cld_db,double rho,int residual,double *result){
 float cld[MAX_PARAMETER_BANDS]={0},icc[MAX_PARAMETER_BANDS]={0};
 float h11[MAX_PARAMETER_BANDS],h12[MAX_PARAMETER_BANDS],h21[MAX_PARAMETER_BANDS],h22[MAX_PARAMETER_BANDS],r1[MAX_PARAMETER_BANDS],r2[MAX_PARAMETER_BANDS],left[MAX_PARAMETER_BANDS],right[MAX_PARAMETER_BANDS];
 cld[0]=(float)cld_db;icc[0]=(float)rho;
 param2UMX_PS_Core(cld,icc,1,residual,h11,h12,h21,h22,r1,r2,left,right);
 result[0]=h11[0];result[1]=h12[0];result[2]=r1[0];result[3]=h21[0];result[4]=h22[0];result[5]=r2[0];result[6]=left[0];result[7]=right[0];
}
