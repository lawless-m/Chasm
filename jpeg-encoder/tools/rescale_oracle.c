/* The PPM-reading oracle for jpeg/ppmread.wack's 12-bit tests: libjpeg-turbo
 * 3.2.0's own rdppm.c, the reader cjpeg uses, compiled at 8 bits through its
 * rdppm-8.c wrapper and linked with the oracle build's libjpeg.a (built
 * -DWITH_SIMD=0). It reads a binary PGM (P5) or PPM (P6) of any maxval and
 * prints the 8-bit samples rdppm.c hands the compressor, one line a pixel
 * row: two-byte big-endian samples for maxval > 255, each through the
 * rescale table start_input_ppm builds. No formula is reimplemented: every
 * value printed is rdppm.c's. A reader error (a sample over maxval, short
 * data) is cjpeg's own message on stderr and a non-zero exit.
 *
 *   rescale-oracle FILE
 *
 * Build, from the repository root (PPM_SUPPORTED switches rdppm.c on, as
 * libjpeg-turbo's own build does for cjpeg):
 *   gcc -O0 -DPPM_SUPPORTED -Itmp/dl/ljt-3.2.0/build -Itmp/dl/ljt-3.2.0/src -o tmp/e3/rescale-oracle jpeg-encoder/tools/rescale_oracle.c tmp/dl/ljt-3.2.0/src/wrapper/rdppm-8.c tmp/dl/ljt-3.2.0/build/libjpeg.a
 */
#include "cdjpeg.h"

#define JMESSAGE(code, string)  string,
static const char * const cdjpeg_message_table[] = {
#include "cderror.h"
  NULL
};

int main(int argc, char **argv)
{
  struct jpeg_compress_struct cinfo;
  struct jpeg_error_mgr jerr;
  cjpeg_source_ptr src;

  if (argc != 2)
    return 2;
  cinfo.err = jpeg_std_error(&jerr);
  jerr.addon_message_table = cdjpeg_message_table;
  jerr.first_addon_message = JMSG_FIRSTADDONCODE;
  jerr.last_addon_message = JMSG_LASTADDONCODE;
  jpeg_create_compress(&cinfo);
  cinfo.in_color_space = JCS_RGB;
  jpeg_set_defaults(&cinfo);
  cinfo.data_precision = 8;
  src = jinit_read_ppm(&cinfo);
  src->input_file = fopen(argv[1], "rb");
  if (src->input_file == NULL)
    return 2;
  src->max_pixels = 0;
  src->start_input(&cinfo, src);
  for (JDIMENSION y = 0; y < cinfo.image_height; y++) {
    src->get_pixel_rows(&cinfo, src);
    JSAMPROW row = src->buffer[0];
    for (JDIMENSION i = 0; i < cinfo.image_width * cinfo.input_components; i++)
      printf(i ? " %d" : "%d", row[i]);
    printf("\n");
  }
  src->finish_input(&cinfo, src);
  fclose(src->input_file);
  jpeg_destroy_compress(&cinfo);
  return 0;
}
