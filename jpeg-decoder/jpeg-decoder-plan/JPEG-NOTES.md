# JPEG notes for the implementer

The specification is ITU-T T.81 (freely available from the ITU as
T.81 (09/92)). Section references below are to it. libjpeg-turbo's source
(jdhuff.c, jdphuff.c, jdarith.c, jidctint.c, jdsample.c, jdcolor.c,
jdmarker.c) is the behavioural reference; when the spec and libjpeg differ
in a corner, match libjpeg, because that is what the oracle does.

## Markers (B.1.1.3)

All markers are `FF xx`, xx not 00 and not FF (FF FF is fill). Segments with
a length have a big-endian 2-byte length that includes itself.

| Marker  | Code  | Notes                                                   |
| ------- | ----- | ------------------------------------------------------- |
| SOI     | D8    | no length                                               |
| EOI     | D9    | no length                                               |
| SOF0    | C0    | baseline Huffman                                        |
| SOF1    | C1    | extended sequential Huffman (12-bit lives here)         |
| SOF2    | C2    | progressive Huffman                                     |
| SOF3    | C3    | lossless: refuse                                        |
| SOF5-7  | C5-C7 | hierarchical: refuse                                    |
| SOF9    | C9    | sequential arithmetic                                   |
| SOF10   | CA    | progressive arithmetic                                  |
| SOF11   | CB    | lossless arithmetic: refuse                             |
| SOF13-15| CD-CF | hierarchical arithmetic: refuse                         |
| DHT     | C4    | Huffman tables                                          |
| DAC     | CC    | arithmetic conditioning                                 |
| RSTn    | D0-D7 | no length; inside entropy data                          |
| SOS     | DA    | followed by entropy data                                |
| DQT     | DB    | quantisation tables                                     |
| DNL     | DC    | number of lines, after the first scan                   |
| DRI     | DD    | restart interval                                        |
| APPn    | E0-EF | APP0 JFIF, APP1 EXIF, APP2 ICC, APP14 Adobe             |
| COM     | FE    | comment                                                 |
| JPG, JPGn, TEM, DHP, EXP | | reserved/hierarchical: refuse            |

Inside entropy data, `FF 00` is a stuffed FF byte. Any other `FF xx` is a
marker: RSTn is consumed by the restart logic; anything else ends the scan.

## Frame header (B.2.2)

Precision P (8 or 12), height Y (0 allowed if DNL follows), width X (not 0),
component count Nf, then per component: id, sampling H (1-4) and V (1-4),
quantisation table index Tq (0-3).

Hmax, Vmax over components. MCU is 8 Hmax by 8 Vmax pixels; a component
contributes H x V blocks per MCU. A non-interleaved scan (one component)
uses that component's own block grid, not MCUs, and the block count is
ceil(ceil(X x H / Hmax) / 8) by ceil(ceil(Y x V / Vmax) / 8): not padded to
whole MCUs. Interleaved scans are padded to whole MCUs. Getting this right
for odd factors is where many decoders fail.

Sampling factor sanity (libjpeg's rule): refuse if any H or V is 0 or > 4;
refuse if Hmax or Vmax is not a multiple of every component's H or V
respectively (libjpeg accepts only factors that divide the max).

## Scan header (B.2.3)

Ns components (1-4), each: component selector Cs, DC table Td, AC table Ta.
Then Ss, Se (spectral selection), Ah, Al (successive approximation). For
sequential: Ss=0, Se=63, Ah=Al=0. Interleaved scans need Ns > 1 and the sum
of H x V over the scan's components <= 10.

## Huffman (Annex C, F.2.2)

DHT: Tc (0 DC, 1 AC), Th (0-3), 16 counts (codes of length 1..16), then the
symbols. Build canonical codes: code = 0; for each length, assign codes in
order, then shift left. Validate: the counts must not over-subscribe
(sum of counts x 2^(16-len) <= 2^16); libjpeg also requires total symbols
<= 256.

Decoding: a 9-bit (libjpeg uses HUFF_LOOKAHEAD = 8) lookup table for short
codes, then the maxcode/valptr walk for longer ones. A code with no symbol
is `BAD_ENTROPY_DATA` (libjpeg warns and uses 0 in lenient mode).

DC: symbol is the magnitude category (0-11 for 8-bit, 0-15 for 12-bit),
followed by that many extra bits; EXTEND (F.2.2.1) sign-extends: if the
first extra bit is 0 the value is negative, value = bits - (2^cat - 1).
Prediction: DC = pred + diff, pred reset to 0 at scan start and each RST.

AC: symbol is RRRRSSSS; RRRR run of zeros, SSSS category. 0x00 EOB,
0xF0 ZRL (16 zeros). Coefficients are placed in zigzag order; dezigzag into
natural order before the IDCT (or run the IDCT on a zigzag-indexed
dequant table, as libjpeg does with its `jpeg_natural_order`).

Standard tables (K.3) are what Motion JPEG streams omit; we refuse a scan
with a missing table (strict), and may optionally supply them (lenient).

## Progressive (Annex G)

Scans carry parts of the coefficients:

- DC first: Ss=Se=0, Ah=0. Decoded DC values are shifted left by Al.
  May be interleaved.
- DC refine: Ss=Se=0, Ah=Al+1. One raw bit per block, ORed in at bit Al.
- AC first: Ss>=1, Ah=0, one component only. Includes EOBn run codes
  (symbol with SSSS=0 and RRRR<15: EOB run of 2^RRRR + extra bits), which
  skip whole blocks.
- AC refine: Ss>=1, Ah=Al+1, one component only. The hardest part:
  G.1.2.3. Correction bits for already-nonzero coefficients are read as
  the decoder passes over them while placing new ±1 coefficients at bit
  Al; EOB runs apply here too and still consume correction bits for
  nonzero coefficients in the skipped blocks. Follow jdphuff.c's
  `decode_mcu_AC_refine` closely.

Validation (libjpeg's rules): spectral bands per component must be
received first-time before refinement; Al must decrease by exactly one in
refinement; Se >= Ss; Ss=0 implies Se=0; Ss>0 requires Ns=1; a DC scan
cannot follow an AC scan for that component's first pass out of order.
Refuse violations as `BAD_PROGRESSION`.

The coefficient store holds every block's 64 coefficients until the last
scan. Output is produced only when EOI is reached (strict) or when input
ends (lenient).

## Arithmetic coding (Annex D, F.1.4, G.1.3)

The QM coder. Decoder state: C register, A register, CT counter, and the
byte-in procedure with stuffing (FF 00) and marker handling (FF xx is
treated as a stream of 1s afterwards). The Qe table (Table D.3) has 113
rows: Qe value, NLPS, NMPS, SWITCH. Each statistics bin is one byte: index
into the table plus the MPS sense.

DC: conditioning on the previous DC difference, bucketed by L and U from
DAC (defaults L=0, U=1). AC: conditioning on coefficient index, with Kx
(default 5) splitting the two magnitude-category contexts. Progressive
arithmetic uses the same bins with the Annex G procedures. jdarith.c is
compact and readable; mirror its structure. The spec's Annex D test data
makes a good unit test before any image is attempted.

Restart resets the coder and all bins.

## IDCT (A.3.3) as libjpeg does it

`jpeg_idct_islow` in jidctint.c: a Loeffler-Ligtenberg-Moschytz 12-multiply
algorithm in fixed point with CONST_BITS=13, PASS1_BITS=2 (8-bit; 1 for
12-bit). Pass 1 down columns, pass 2 across rows, with a shortcut when a
column's AC terms are all zero. Final: add 128 (or 2048) and range-limit.
Dequantisation is a straight multiply by the quant table entry before the
IDCT (libjpeg folds it into the "multiplier table" per component).

The range-limit table trick: an array of 5 x (MAX+1) entries lets an
out-of-range result index into a clamped region without a branch. Replicate
or use explicit min/max; results must be identical.

## Upsampling (jdsample.c)

With `-nosmooth`: replication (each chroma sample repeated H and V times).

"Fancy" (the default): h2v1 uses 3/4 nearest + 1/4 next, with the rounding
alternating (+1 and +2 before the >>2) between the pair; h2v2 does the
triangle in both directions with 9/16, 3/16, 3/16, 1/16 weights via the
two-step method (vertical first into a temporary row, then horizontal),
again with alternating rounding. Edge columns and rows replicate. Only
h2v1 and h2v2 are fancy in libjpeg; other factors fall back to replication.
libjpeg-turbo also has h1v2 fancy upsampling; check the pinned version and
match it.

Upsampling is done per MCU row with context rows for h2v2 (the row above
and below), which is why the MCU-row driver needs a one-row lookahead for
sequential decoding.

## Colour (jdcolor.c)

Decision tree (jdapimin.c `default_decompress_parms`):

- 1 component: greyscale.
- 3 components: JFIF marker seen -> YCbCr. Else Adobe marker seen:
  transform 0 -> RGB, 1 -> YCbCr, else YCbCr with a warning. Else component
  ids 1,2,3 -> YCbCr; ids 'R','G','B' (82,71,66) -> RGB; otherwise YCbCr.
- 4 components: Adobe transform 0 -> CMYK, 2 -> YCCK, else YCCK with a
  warning. No Adobe marker -> CMYK.

YCbCr -> RGB fixed point: SCALEBITS 16, ONE_HALF = 1 << 15, FIX(x) =
round(x x 65536). R = Y + Cr_r[Cr], G = Y + ((Cb_g[Cb] + Cr_g[Cr]) >> 16),
B = Y + Cb_b[Cb], with Cr_r = FIX(1.40200) x (Cr-128), Cb_b = FIX(1.77200)
x (Cb-128), Cr_g = -FIX(0.71414) x (Cr-128), Cb_g = -FIX(0.34414) x
(Cb-128) + ONE_HALF; each rounded with ONE_HALF then >> 16; then range
limited. YCCK -> CMYK converts the first three channels the same way and
then inverts (MAX - value), K unchanged.

Adobe CMYK files store inverted values (Photoshop writes CMYK as
"inverted", i.e. 255 means no ink). libjpeg itself does not invert; Pillow
and most viewers invert when the Adobe marker is present. For our PPM
(viewing only) apply the inversion when Adobe is present, then
R = (255-C) x (255-K) / 255 etc., a naive conversion that looks right. For
the raw plane dump, store the samples exactly as libjpeg produces them
(uninverted), and compare against Pillow configured to match (or apply the
same inversion on both sides; just be explicit about which).

## 12-bit

Samples 0-4095, centre 2048. DC categories up to 15 (Huffman) and larger
magnitude ranges for the arithmetic conditioning. islow uses PASS1_BITS=1.
Quantisation tables may be 16-bit (Pq=1). The range limit table scales
accordingly. PPM output: maxval 4095, samples as 2 bytes big-endian.

## Things that routinely go wrong

- Non-interleaved scan block counts for subsampled components (not padded
  to MCUs) versus interleaved (padded).
- Forgetting that DNL may define the height after the first scan.
- Byte stuffing inside the arithmetic coder's byte-in (it is different
  from Huffman's: after a marker the coder feeds zeros/ones, see D.2.6).
- AC refinement's handling of EOB runs with pending correction bits.
- Restart intervals in progressive scans count MCUs in the scan's own
  terms (blocks for non-interleaved scans).
- h2v2 fancy upsampling at the top and bottom rows of the image and at
  MCU-row boundaries (context rows).
- The rounding offsets in both fancy upsampling and colour conversion.
  Off by one everywhere is the signature of getting these wrong.
- 16-bit DQT entries, and DQT/DHT segments defining several tables at once.
- Sampling factors where a component's H does not divide Hmax (refuse, as
  libjpeg does).
