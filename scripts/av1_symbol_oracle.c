/* Test-only libaom encoder oracle; never linked into FVid.
 * Build on macOS: cc scripts/av1_symbol_oracle.c /opt/homebrew/lib/libaom.a
 *   -lm -lpthread -o /tmp/fvid-av1-symbol-oracle
 * Run: /tmp/fvid-av1-symbol-oracle > tests/fixtures/av1/symbols.bin
 * The ABI declarations match libaom 3.15 aom_dsp/entenc.h.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef struct {
  unsigned char *buf;
  uint32_t storage, offs;
  uint64_t low;
  uint16_t rng;
  int16_t cnt;
  int error;
} Encoder;
extern void od_ec_enc_init(Encoder *, uint32_t);
extern void od_ec_enc_clear(Encoder *);
extern void od_ec_encode_cdf_q15(Encoder *, int, const uint16_t *, int);
extern unsigned char *od_ec_enc_done(Encoder *, uint32_t *);
static void u32le(uint32_t x) {
  for (int i=0;i<4;i++) putchar((x>>(i*8))&255);
}
int main(void) {
  for (int adaptive=0;adaptive<2;adaptive++) {
    for (int n=2;n<=16;n++) {
      Encoder enc;
      od_ec_enc_init(&enc, 65536);
      uint16_t cdf[17];
      for (int i=0;i<n;i++) cdf[i]=32768-(32768*(i+1)/n);
      cdf[n]=0;
      uint32_t state=0x31415926;
      for (int k=0;k<1024;k++) {
        state=state*1664525u+1013904223u;
        int symbol=(state>>16)%n;
        od_ec_encode_cdf_q15(&enc, symbol, cdf, n);
        if (adaptive) {
          unsigned rate=3+(cdf[n]>15)+(cdf[n]>31)+(n>=4?2:1);
          for (int i=0;i<n-1;i++) {
            if (i<symbol) cdf[i]+=(32768-cdf[i])>>rate;
            else cdf[i]-=cdf[i]>>rate;
          }
          if (cdf[n]<32) cdf[n]++;
        }
      }
      uint32_t bytes;
      unsigned char *out=od_ec_enc_done(&enc,&bytes);
      if (!out || enc.error) return 1;
      u32le(bytes);
      fwrite(out,1,bytes,stdout);
      od_ec_enc_clear(&enc);
    }
  }
  return ferror(stdout)?1:0;
}
