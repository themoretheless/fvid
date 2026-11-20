/* Research-only caller; original ISO implementation is external and unchanged. */
#include <stdio.h>
#include <stdlib.h>
#include "arbitrary-ebq-source.c"
int debug[256];
void myexit(char *message) { fprintf(stderr, "%s\n", message); exit(7); }
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    spatialDec *d = calloc(1, sizeof(*d));
    if (!d) return 3;
    d->quantMode = atoi(argv[1]);
    d->numOttBoxes = 1;
    d->numParameterSets = 1;
    d->paramSlot[0] = 7;
    d->bsConfig.numOttBoxesAT = 1;
    d->bsConfig.bsOttBandsAT[0] = 23;
    d->bsConfig.bsOttDefaultCldAT[0] = 15;
    /* Zero-initialized lossless state selects legal default data mode. */
    decodeAndMapFrameArbTree(d);
    printf("quant=%d default_cld=%g\n", d->quantMode, d->ottCLD[1][0][0]);
    free(d);
    return 0;
}
