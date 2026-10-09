# Language feedback

What Whackford makes awkward while writing the encoder: present-tense facts,
each with what the encoder does about it.

- `bytes` has little-endian `bytes.u16-at!` and `bytes.u32-at!` but no
  big-endian write, so every 16-bit JPEG header field (marker lengths,
  dimensions) is written as two separate bytes.
- There is no `bytes` literal and no `str`-to-`bytes` word short of
  `bytes.put` into a fresh buffer, so tests build buffers from hex strings
  with the decoder's `hex` helper. That is why the encoder's programs list
  `../jpeg-decoder/jpeg/limits.wack`, `refuse.wack` and `fixtures.wack`
  first, used unchanged for `refusal ( str str -- str )` and
  `hex ( str -- bytes )`.
- There is no array literal, so constant tables (quantisation, zigzag,
  Huffman counts and symbols) are built at run time from hex strings into an
  `array i32`, as the decoder's `idct.make` does.
- `wack test` exits 0 even with pending tests, so gates grep the summary
  line for `0 failed, 0 pending`.
