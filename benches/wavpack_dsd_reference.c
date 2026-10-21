/* Optional reference-only executable. Never linked into production or tests. */
#include "wavpack.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc,char **argv){
 if(argc!=5 && argc!=6)return 2;
 int pcm=strcmp(argv[4],"pcm")==0;char error[256];
 WavpackContext *w=WavpackOpenFileInput(argv[1],error,pcm?OPEN_DSD_AS_PCM:OPEN_DSD_NATIVE,0);
 if(!w){fprintf(stderr,"%s\n",error);return 3;}
 unsigned n=(unsigned)strtoul(argv[3],NULL,10);int channels=WavpackGetNumChannels(w);
 if(!n||channels<1||channels>4096)return 4;
 int32_t *buffer=calloc((size_t)n*channels,sizeof(*buffer));if(!buffer)return 5;
 unsigned chunk=argc==6?(unsigned)strtoul(argv[5],NULL,10):n;
 if(!chunk)return 4;
 unsigned count=0;while(count<n){unsigned request=n-count;if(request>chunk)request=chunk;unsigned got=WavpackUnpackSamples(w,buffer+(size_t)count*channels,request);count+=got;if(got!=request)break;}
 if(count!=n||WavpackGetNumErrors(w)){fprintf(stderr,"frames=%u/%u CRC errors=%d\n",count,n,WavpackGetNumErrors(w));return 6;}
 FILE *out=fopen(argv[2],"wb");if(!out)return 7;
 for(size_t i=0;i<(size_t)n*channels;i++){
  if(pcm){float sample=(float)buffer[i]*(1.0f/8388608.0f);if(fwrite(&sample,4,1,out)!=1)return 8;}
  else {unsigned char sample=(unsigned char)buffer[i];if(fwrite(&sample,1,1,out)!=1)return 8;}
 }
 int failed=fclose(out);free(buffer);WavpackCloseFile(w);return failed?9:0;
}
