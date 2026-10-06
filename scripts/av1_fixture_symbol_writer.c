/* Fixture-only range encoder. Build with libaom.a like av1_symbol_oracle.c.
 * stdin: alphabet symbol then ascending cumulative probabilities, one record
 * per symbol. No FVid runtime or ordinary test links this external reference. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef struct { unsigned char *buf; uint32_t storage,offs; uint64_t low; uint16_t rng; int16_t cnt; int error; } Encoder;
extern void od_ec_enc_init(Encoder *,uint32_t);
extern void od_ec_enc_clear(Encoder *);
extern void od_ec_encode_cdf_q15(Encoder *,int,const uint16_t *,int);
extern unsigned char *od_ec_enc_done(Encoder *,uint32_t *);
int main(void) {
 Encoder e;od_ec_enc_init(&e,65536);int n,s,previous;
 while(scanf("%d",&n)==1) {
  if(n<2||n>16||scanf("%d",&s)!=1||s<0||s>=n)return 2;
  uint16_t cdf[17]={0};previous=0;
  for(int i=0;i<n;i++){int p;if(scanf("%d",&p)!=1||p<previous||p>32768)return 2;previous=p;cdf[i]=32768-p;}
  if(previous!=32768)return 2;od_ec_encode_cdf_q15(&e,s,cdf,n);
 }
 uint32_t bytes;unsigned char *out=od_ec_enc_done(&e,&bytes);
 if(!out||e.error)return 1;int failed=fwrite(out,1,bytes,stdout)!=bytes;
 od_ec_enc_clear(&e);return failed?1:0;
}
