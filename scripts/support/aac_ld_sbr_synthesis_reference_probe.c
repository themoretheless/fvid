/* Optional source-reference driver. No archived implementation is copied here. */
#include <stdio.h>
#include "ct_sbrconst.h"
#ifdef FVID_PRECISE_PI
#undef PI
#define PI 3.141592653589793238462643383279502884
#endif
#include "ct_polyphase.c"
int main(int argc, char **argv) {
  if (argc != 2) return 2;
  FILE *out = fopen(argv[1], "wb");
  if (!out) return 3;
  for (int down = 0; down <= 1; ++down) {
    InitSbrSynFilterbank(down, FBMODE_LDQMF);
    int bands = down ? 32 : 64;
    for (int n = 0; n < 2 * bands; ++n)
      for (int k = 0; k < bands; ++k) {
        double value[2] = {N.real[n][k], N.imag[n][k]};
        if (fwrite(value, sizeof(double), 2, out) != 2) return 4;
      }
  }
  return fclose(out) != 0;
}
