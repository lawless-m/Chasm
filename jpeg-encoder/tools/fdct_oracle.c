/* The forward DCT oracle for jpeg/fdct.wack's tests: libjpeg-turbo 3.2.0's
 * own jpeg_fdct_islow (jfdctint.c, the source the oracle cjpeg was built
 * from). Reads 64 samples (0 to 255, row-major) from stdin, level-shifts
 * them by 128 as jcdctmgr.c convsamp does, runs the DCT and prints the 64
 * coefficients on one line, separated by single spaces.
 *
 * Build, from the repository root:
 *   gcc -O0 -Itmp/dl/ljt-3.2.0/build -Itmp/dl/ljt-3.2.0/src -o tmp/e1/fdct-oracle jpeg-encoder/tools/fdct_oracle.c tmp/dl/ljt-3.2.0/src/jfdctint.c
 */
#include <stdio.h>
#define JPEG_INTERNALS
#include "jinclude.h"
#include "jpeglib.h"
#include "jdct.h"
#include "jsamplecomp.h"

int main(void)
{
  int d[64];
  for (int i = 0; i < 64; i++) {
    int v;
    if (scanf("%d", &v) != 1)
      return 1;
    d[i] = v - 128;
  }
  _jpeg_fdct_islow(d);
  for (int i = 0; i < 64; i++)
    printf(i ? " %d" : "%d", d[i]);
  printf("\n");
  return 0;
}
