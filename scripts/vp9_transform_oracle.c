// Test-only fixture generator; libvpx is never linked into FVid.
// cc scripts/vp9_transform_oracle.c /opt/homebrew/lib/libvpx.a -o /tmp/vp9-oracle
// /tmp/vp9-oracle > tests/fixtures/vp9/transforms.bin
#include <stdint.h>
#include <stdio.h>
#include <string.h>
extern void vp9_iht4x4_16_add_c(const int32_t*,uint8_t*,int,int);
extern void vp9_iht8x8_64_add_c(const int32_t*,uint8_t*,int,int);
extern void vp9_iht16x16_256_add_c(const int32_t*,uint8_t*,int,int);
extern void vpx_idct32x32_1024_add_c(const int32_t*,uint8_t*,int);
extern void vpx_iwht4x4_16_add_c(const int32_t*,uint8_t*,int);
int main(void) {
 uint32_t rng=0x73ab9215;
 for(int size=4;size<=32;size*=2) for(int kind=0;kind<(size==4?5:size==32?1:4);kind++) for(int test=0;test<32;test++) {
  int32_t coefficients[1024]; uint8_t pixels[1024];
  memset(pixels,128,sizeof pixels);
  for(int i=0;i<size*size;i++) {rng=rng*1664525u+1013904223u; coefficients[i]=(int32_t)((rng>>16)%65)-32;}
  if(kind==4) vpx_iwht4x4_16_add_c(coefficients,pixels,size);
  else if(size==4) vp9_iht4x4_16_add_c(coefficients,pixels,size,kind);
  else if(size==8) vp9_iht8x8_64_add_c(coefficients,pixels,size,kind);
  else if(size==16) vp9_iht16x16_256_add_c(coefficients,pixels,size,kind);
  else vpx_idct32x32_1024_add_c(coefficients,pixels,size);
  if(fwrite(pixels,1,size*size,stdout)!=(size_t)(size*size)) return 1;
 }
 return 0;
}
