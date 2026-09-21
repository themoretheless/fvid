/* Test-only independent libaom integer transform oracle. */
#include <stdint.h>
#include <stdio.h>
#define DECL(W,H) extern void av1_inv_txfm2d_add_##W##x##H##_c(const int32_t*,uint16_t*,int,int,int);
DECL(4,4) DECL(8,8) DECL(16,16) DECL(32,32) DECL(64,64)
DECL(4,8) DECL(8,4) DECL(8,16) DECL(16,8) DECL(16,32) DECL(32,16)
DECL(32,64) DECL(64,32) DECL(4,16) DECL(16,4) DECL(8,32) DECL(32,8) DECL(16,64) DECL(64,16)
typedef void (*Transform)(const int32_t*,uint16_t*,int,int,int);
#define FN(W,H) av1_inv_txfm2d_add_##W##x##H##_c
int main(void) {
  const int widths[]={4,8,16,32,64,4,8,8,16,16,32,32,64,4,16,8,32,16,64};
  const int heights[]={4,8,16,32,64,8,4,16,8,32,16,64,32,16,4,32,8,64,16};
  const Transform transforms[]={FN(4,4),FN(8,8),FN(16,16),FN(32,32),FN(64,64),FN(4,8),FN(8,4),FN(8,16),FN(16,8),FN(16,32),FN(32,16),FN(32,64),FN(64,32),FN(4,16),FN(16,4),FN(8,32),FN(32,8),FN(16,64),FN(64,16)};
  for(int size=0;size<19;size++) {
    int w=widths[size],h=heights[size];
    for(int type=0;type<16;type++) {
      if((w==64||h==64)&&type!=0)continue;
      if((w==32||h==32)&&type!=0&&type!=9)continue;
      for(int depth=8;depth<=12;depth+=2) for(int trial=0;trial<4;trial++) {
        int32_t input[4096]={0};uint16_t output[4096];
        uint32_t rng=0x73915926u+trial;
        for(int y=0;y<h;y++)for(int x=0;x<w;x++) {
          rng=rng*1664525u+1013904223u;
          if(x<32&&y<32) input[x*(h<32?h:32)+y]=(int)((rng>>16)%1025)-512;
          output[y*w+x]=1<<(depth-1);
        }
        transforms[size](input,output,w,type,depth);
        for(int i=0;i<w*h;i++){putchar(output[i]&255);putchar(output[i]>>8);}
      }
    }
  }
  return ferror(stdout)?1:0;
}
