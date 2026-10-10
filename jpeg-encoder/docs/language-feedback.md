# Language feedback

What Whackford makes awkward while writing the encoder: present-tense facts,
each with what the encoder does about it.

- `bytes` has little-endian `bytes.u16-at!` and `bytes.u32-at!` but no
  big-endian write, so every 16-bit JPEG header field (marker lengths,
  dimensions) is written as two separate bytes.
- There is no `bytes` literal and no `str`-to-`bytes` word short of
  `bytes.put` into a fresh buffer, so tests build buffers from hex strings
  with the shared `hex` helper. That is one reason every program lists
  `../jpeg-shared/limits.wack`, `refuse.wack` and `fixtures.wack` first,
  for `refusal ( str str -- str )`, `hex ( str -- bytes )` and
  `hex-array ( str -- array i32 )`.
- There is no array literal, so constant tables (quantisation, zigzag,
  Huffman counts and symbols) are built at run time from hex strings into an
  `array i32`, as the decoder's `idct.make` does.
- `wack test` exits 0 even with pending tests, so gates grep the summary
  line for `0 failed, 0 pending`.
- A stack assertion asserts the whole stack, so `64 array.new ( array i32 )`
  fails when other values sit beneath it (encoder.on builds its struct's
  fields on the stack). The array is bound to a local first, then pushed.
- `bytes.u16-at` reads little-endian only, so the two-byte big-endian
  samples of a 12-bit PPM are read as two `bytes.at` and combined
  (ppmread.narrow).
- There is no way to learn a file's size before reading it, so
  encoder.load-icc reads an ICC profile into one buffer of MAX_ICC + 1
  bytes (1 MB) whatever the profile's size.

## Speed

Measured with `tmp/speed/timing.py` and a release compiler built into
`tmp/speed/target` (wasmtime, bounds checks on, compiles cached). A row is
one warm-up run, then five timed runs; its CPU is the median child CPU time
(user plus system, from `wait4`), which is what is judged, because a
40-job decoder fuzz ran on the same 40-core machine throughout, at load
averages between 25 and 40, recorded beside every figure. The table is the
per-row smaller median of two passes (`tmp/speed/final.txt`); the two
passes differ by up to 18% on a row, so a change is judged by an
interleaved A/B instead (`tmp/speed/ab.py`: the snapshot and the working
tree run alternately, seven pairs, both outputs compared byte for byte),
whose A-against-A spread is under 3%, and kept only when its target row is
faster by more than 5.4% (twice that spread) with no row slower by as much.

The inputs (`tmp/speed/mkinputs.py`): `photo12.ppm` is
`primary/testorig.jpg`'s decode tiled to 4000x3000 (12 megapixels) and
`photo12g.pgm` its greyscale; `pattern6-12.ppm` and `p12bit.ppm` (maxval
4095) are `tools/gen_ppm.py pattern6` at 4000x3000; `big12.jpg` is the
oracle's cjpeg of `photo12.ppm` at q85 4:2:0, `big12-p12.jpg` the 12-bit
pattern as a 12-bit SOF1 file, and `big80.jpg` the same tiling at
9600x8000 (76.8 megapixels); the `-o6` files carry an Exif orientation 6.
Every output is byte-exact against cjpeg 3.2.0 `-dct int -baseline`, and
clean's against cjpeg of `tools/orient.py` of djpeg's decode; the cjpeg
column is that cjpeg alone (for clean rows, without djpeg's decode). The
rate is the megapixels over the CPU less the startup (0.011 s for encode,
0.020 s for clean, a 1x1 image).

| Row | Program | Input | CPU | Wall | cjpeg | Megapixels/s | Baseline CPU |
|---|---|---|---|---|---|---|---|
| photo12-q85-420 | encode | photo12.ppm, q85 4:2:0 | 0.748 s | 0.761 s | 0.276 s | 16.3 | 0.756 s |
| photo12-q85-444 | encode | photo12.ppm, q85 4:4:4 | 1.129 s | 1.150 s | 0.467 s | 10.7 | 1.345 s |
| photo12-q40-420 | encode | photo12.ppm, q40 4:2:0 | 0.664 s | 0.664 s | 0.253 s | 18.4 | 0.730 s |
| photo12g-q85 | encode | photo12g.pgm, q85 grey | 0.388 s | 0.388 s | 0.149 s | 31.8 | 0.425 s |
| pattern6-q85-420 | encode | pattern6-12.ppm, q85 4:2:0 | 0.657 s | 0.821 s | 0.260 s | 18.6 | 0.701 s |
| p12bit-q85-420 | encode | p12bit.ppm (maxval 4095), q85 4:2:0 | 0.825 s | 0.867 s | 0.299 s | 14.7 | 0.836 s |
| big12-q85-420 | clean | big12.jpg, q85 4:2:0 | 1.217 s | 1.322 s | 0.272 s | 10.0 | 1.291 s |
| big12-o6-q85-420 | clean | big12-o6.jpg (orientation 6), q85 4:2:0 | 1.291 s | 1.390 s | 0.307 s | 9.4 | 1.488 s |
| big12-p12-q85-420 | clean | big12-p12.jpg (12-bit SOF1), q85 4:2:0 | 1.407 s | 1.580 s | 0.341 s | 8.7 | 1.469 s |
| big80-q85-420 | clean | big80.jpg, q85 4:2:0 | 7.507 s | 7.507 s | 1.841 s | 10.3 | 7.897 s |
| big80-o6-q85-420 | clean | big80-o6.jpg (orientation 6), q85 4:2:0 | 8.395 s | 8.396 s | 2.013 s | 9.2 | 9.016 s |

Where the time goes (removal breakdowns in scratch copies,
`tmp/speed/stages-encode-final.txt` and `stages-clean-final.txt`):

- An encode of `photo12.ppm` at q85 4:2:0: Huffman coding 29%,
  quantisation 22%, colour conversion 20%, the FDCT 14%, loading each 8x8
  block from the component planes 7%, downsampling 3%, the rest (PPM
  reading, padding, markers) 4%, startup 2%. Writing the output costs
  nothing measurable: to `/dev/null` it measured within noise of a real
  file.
- A clean of `big12.jpg`: the decoder 37%, the encoder 62%. With
  orientation 6, the decode and the gather 37%, the turn 9%, the encode
  53%.

Kept:

- `orient.row-into` reads by source stride: it computes, once per output row, the source pixel of its
  first pixel and the source step between neighbours (`orient.start`);
  orientations 1 and 4 copy a whole source row with one `bytes.put`, the
  others step through the image with one loop of three byte copies a pixel
  (one for grey), with no per-pixel test of the orientation. A/B:
  big12-o6 -7.8%, big80-o6 -7.2%, big12 (orientation 1, the control)
  -1.6%, at load 30 to 34. The turn was 14% of an orientation-6 clean and
  is 9%.
- `fdct.block` is `fdct.rows` then `fdct.cols`: each loads its line's eight
  values at constant offsets and stores them with its pass's fixed shift
  or descale, where one `fdct.line` took the pass as a parameter, tested
  it three times a line and multiplied out every element's index. A/B:
  photo12g -8.9%, photo12 -3.4%, pattern6 +0.4%, big12 +0.8%, at load 32
  to 33. The FDCT was 21% of a colour encode and is 14%.

Tried and not kept:

- Quantisation by libjpeg-turbo's reciprocals (jcdctmgr.c
  compute_reciprocal and quantize): a 192-entry divisor table per
  quantisation table and a 64-bit multiply and shift per coefficient in
  place of one `i32.div_s`. Byte-exact, and `tmp/speed/recip_check.py`
  shows the arithmetic equal to the rounded division for every divisor and
  every magnitude up to 16384, but the target row photo12 q85 4:2:0
  measured +1.4% (photo12 q40 -6.9%, grey -7.3%, big12 -4.0%), so the rule
  did not keep it. The engine runs `i32.div_s` as a hardware divide; the
  replacement adds two extends, three array reads and a 64-bit multiply.

Baseline to final, from separate timing passes under the fuzz's load: one
final pair gave -1% to -16% per row and -6.7% on the sum of the eleven
rows (25.95 s to 24.23 s); a later pair gave -1.8% on the sum (25.49 s),
every row exact and none slower by more than 5.4%. The passes' own spread
(up to 18% on a row) is larger than the end-to-end change, so the total is
within noise under this load; the A/B figures above are the measured
effect of each change, a few percent overall, mostly on orientation-6 and
greyscale images.

Findings about Whackford:

- Every `bytes.at` and `array.at` is bounds-checked, and the hot words
  (Huffman coding, the block load) are mostly loads, so they pay for it:
  the encoder spends about 3x cjpeg's CPU on the same image.
- A word that takes a mode parameter and branches on it inside a loop
  (`fdct.line`'s pass) is not specialised by the compiler; writing the two
  passes out is what removed the tests.
- There is no array literal, so constant tables (quantisation, Huffman,
  the natural order) are built from hex strings once per encode; the
  divisor tables would have been the same.
