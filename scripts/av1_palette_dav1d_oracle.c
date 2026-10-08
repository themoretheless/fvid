/* Optional second pixel oracle for owned single- or multi-frame AV1 fixtures.
 * Build with pkg-config dav1d; not linked to FVid or ordinary tests. */
#include <dav1d/dav1d.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <errno.h>
int main(int argc,char **argv) {
 if(argc!=3&&argc!=4)return 2;
 int expected=argc==4?atoi(argv[3]):1;if(expected<1)return 2;
 FILE *input=fopen(argv[1],"rb");if(!input)return 2;
 if(fseek(input,0,SEEK_END))return 2;long length=ftell(input);if(length<=0)return 2;rewind(input);
 Dav1dData data={0};uint8_t *bytes=dav1d_data_create(&data,(size_t)length);if(!bytes||fread(bytes,1,(size_t)length,input)!=(size_t)length)return 2;fclose(input);
 Dav1dSettings settings;dav1d_default_settings(&settings);settings.n_threads=1;settings.max_frame_delay=1;settings.apply_grain=0;
 Dav1dContext *context=NULL;if(dav1d_open(&context,&settings))return 2;
 FILE *output=fopen(argv[2],"wb");if(!output)return 2;
 int written=0;while(written<expected) {
 if(data.sz) {int rc=dav1d_send_data(context,&data);if(rc&&rc!=-EAGAIN)return 1;}
 Dav1dPicture picture={0};int rc=dav1d_get_picture(context,&picture);
 if(rc==-EAGAIN) {if(!data.sz)return 1;continue;}if(rc)return 1;
 if(picture.p.layout!=DAV1D_PIXEL_LAYOUT_I420)return 1;
 for(int p=0;p<3;p++) {
  int w=p?(picture.p.w+1)/2:picture.p.w,h=p?(picture.p.h+1)/2:picture.p.h;
  for(int row=0;row<h;row++)for(int col=0;col<w;col++) {
   const uint8_t *start=(const uint8_t *)picture.data[p]+row*picture.stride[p?1:0];
   unsigned value=picture.p.bpc>8?((const uint16_t *)start)[col]:start[col];
   if(fputc(value&255,output)==EOF||(picture.p.bpc>8&&fputc(value>>8,output)==EOF))return 2;
  }
 }
 dav1d_picture_unref(&picture);written++;
 }
 if(fclose(output))return 2;dav1d_data_unref(&data);dav1d_close(&context);return 0;
}
