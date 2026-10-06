/* External libaom reference for owned AV1 fixtures: flat check or YUV output. */
#include <aom/aom_decoder.h>
#include <aom/aomdx.h>
#include <stdio.h>
#include <stdlib.h>
int main(int argc,char **argv){
 if(argc!=3&&argc!=4)return 2;
 FILE *output=argc==4?fopen(argv[3],"wb"):NULL;if(argc==4&&!output)return 2;
 FILE *f=fopen(argv[1],"rb");if(!f)return 2;fseek(f,0,SEEK_END);long n=ftell(f);rewind(f);unsigned char *data=malloc(n);if(!data||fread(data,1,n,f)!=(size_t)n)return 2;fclose(f);
 aom_codec_ctx_t ctx={0};if(aom_codec_dec_init(&ctx,aom_codec_av1_dx(),NULL,0))return 2;
 size_t at=0,packet_start=0;int count=0;
 while(at<(size_t)n){unsigned header=data[at++];if(header&4)at++;size_t size=0;unsigned shift=0;
  do{if(at>=(size_t)n||shift>56)return 2;unsigned byte=data[at++];size|=(size_t)(byte&127)<<shift;shift+=7;if(!(byte&128))break;}while(1);
  if(size>(size_t)n-at)return 2;at+=size;
  if((header>>3)!=3&&(header>>3)!=6)continue;
  if(aom_codec_decode(&ctx,data+packet_start,at-packet_start,NULL)){fprintf(stderr,"%s: %s %s\n",argv[1],aom_codec_error(&ctx),aom_codec_error_detail(&ctx));return 1;}
  packet_start=at;
  aom_codec_iter_t iter=NULL;aom_image_t *img;
  while((img=aom_codec_get_frame(&ctx,&iter))){if(img->d_w!=32||img->d_h!=32||img->bit_depth!=8)return 1;
   for(int p=0;p<3;p++)for(int y=0;y<(p?16:32);y++)for(int x=0;x<(p?16:32);x++){unsigned char value=img->planes[p][y*img->stride[p]+x];if(output){if(fputc(value,output)==EOF)return 2;}else if(value!=128){fprintf(stderr,"non-flat sample\n");return 1;}}count++;
  }
 }
 if(output&&fclose(output))return 2;
 aom_codec_destroy(&ctx);free(data);if(count!=atoi(argv[2])){fprintf(stderr,"frames %d expected %s\n",count,argv[2]);return 1;}return 0;
}
