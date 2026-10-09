/* The colour-conversion and downsampling oracle for jpeg/ccolour.wack's and
 * jpeg/downsample.wack's tests: libjpeg-turbo 3.2.0's own rgb_ycc_convert
 * (jccolor.c) and h2v2/h2v1/fullsize downsamplers with expand_right_edge
 * (jcsample.c), reached through the library's public setup and linked from
 * the oracle build's libjpeg.a (built -DWITH_SIMD=0, the code cjpeg runs).
 * No formula is reimplemented here: every value printed is the library's.
 *
 *   prep-oracle ycc W        reads W pixels as R G B; prints the W Y, the
 *                            W Cb and the W Cr values, one line each
 *   prep-oracle down W HxV   reads V rows of W samples of one plane, fed
 *                            as all three planes with component 0 sampled
 *                            HxV and 1 and 2 at 1x1; prints component 0's
 *                            V output rows, then component 1's and 2's one
 *                            row each, every row width_in_blocks x 8 long
 *
 * Build, from the repository root:
 *   gcc -O0 -Itmp/dl/ljt-3.2.0/build -Itmp/dl/ljt-3.2.0/src -o tmp/e2/prep-oracle jpeg-encoder/tools/prep_oracle.c tmp/dl/ljt-3.2.0/build/libjpeg.a
 */
#define JPEG_INTERNALS
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "jinclude.h"
#include "jpeglib.h"

static void print_row(JSAMPROW row, int n)
{
  for (int i = 0; i < n; i++)
    printf(i ? " %d" : "%d", row[i]);
  printf("\n");
}

static int read_int(void)
{
  int v;
  if (scanf("%d", &v) != 1)
    exit(1);
  return v;
}

int main(int argc, char **argv)
{
  struct jpeg_compress_struct cinfo;
  struct jpeg_error_mgr jerr;
  unsigned char *mem = NULL;
  unsigned long memsize = 0;
  int ycc, w, h = 2, v = 2;

  if (argc < 3)
    return 2;
  ycc = strcmp(argv[1], "ycc") == 0;
  w = atoi(argv[2]);
  if (!ycc && (argc < 4 || sscanf(argv[3], "%dx%d", &h, &v) != 2))
    return 2;

  cinfo.err = jpeg_std_error(&jerr);
  jpeg_create_compress(&cinfo);
  jpeg_mem_dest(&cinfo, &mem, &memsize);
  cinfo.image_width = w;
  cinfo.image_height = 16;
  cinfo.input_components = 3;
  cinfo.in_color_space = JCS_RGB;
  jpeg_set_defaults(&cinfo);
  cinfo.comp_info[0].h_samp_factor = h;
  cinfo.comp_info[0].v_samp_factor = v;
  jpeg_start_compress(&cinfo, TRUE);

  if (ycc) {
    JSAMPROW in = malloc(w * 3);
    JSAMPROW out[3];
    JSAMPARRAY planes[3];
    for (int i = 0; i < w * 3; i++)
      in[i] = read_int();
    for (int c = 0; c < 3; c++) {
      out[c] = malloc(w);
      planes[c] = &out[c];
    }
    (*cinfo.cconvert->color_convert) (&cinfo, &in, planes, 0, 1);
    for (int c = 0; c < 3; c++)
      print_row(out[c], w);
  } else {
    int vmax = cinfo.max_v_samp_factor;
    JSAMPROW rows[4];
    JSAMPARRAY in[3], out[3];
    for (int r = 0; r < vmax; r++) {
      jpeg_component_info *c0 = &cinfo.comp_info[0];
      size_t len = c0->width_in_blocks * 8 * cinfo.max_h_samp_factor / c0->h_samp_factor;
      for (int c = 1; c < 3; c++) {
        jpeg_component_info *ci = &cinfo.comp_info[c];
        size_t l = ci->width_in_blocks * 8 * cinfo.max_h_samp_factor / ci->h_samp_factor;
        if (l > len)
          len = l;
      }
      rows[r] = calloc(len, 1);
      for (int x = 0; x < w; x++)
        rows[r][x] = read_int();
    }
    for (int c = 0; c < 3; c++) {
      jpeg_component_info *ci = &cinfo.comp_info[c];
      /* every plane gets its own copy: expand_right_edge writes in place */
      in[c] = malloc(sizeof(JSAMPROW) * vmax);
      for (int r = 0; r < vmax; r++) {
        size_t len = ci->width_in_blocks * 8 * cinfo.max_h_samp_factor / ci->h_samp_factor;
        size_t l0 = cinfo.comp_info[0].width_in_blocks * 8 * cinfo.max_h_samp_factor / cinfo.comp_info[0].h_samp_factor;
        if (l0 > len)
          len = l0;
        in[c][r] = calloc(len, 1);
        memcpy(in[c][r], rows[r], w);
      }
      out[c] = malloc(sizeof(JSAMPROW) * ci->v_samp_factor);
      for (int r = 0; r < ci->v_samp_factor; r++)
        out[c][r] = calloc(ci->width_in_blocks * 8, 1);
    }
    (*cinfo.downsample->downsample) (&cinfo, in, 0, out, 0);
    for (int c = 0; c < 3; c++) {
      jpeg_component_info *ci = &cinfo.comp_info[c];
      for (int r = 0; r < ci->v_samp_factor; r++)
        print_row(out[c][r], ci->width_in_blocks * 8);
    }
  }
  jpeg_destroy_compress(&cinfo);
  free(mem);
  return 0;
}
