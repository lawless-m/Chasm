# corpus/regressions/

Every input that made the decoder trap, hang, exceed the harness's CPU
budget or hit its memory cap: the classes `trap`, `hang`, `cpu`, `memory`
and `odd` of `tools/harness.py`. `tools/mutate.py`, the mutation fuzzer,
saves each one here as

    <source stem>-<mutation kinds joined by '-'>-<first 8 hex digits of the input's SHA-1>.jpg

with a note of the same name ending `.txt` beside it. The note has one
`key: value` per line:

- `class:` the harness class when the input was found
- `source:` the corpus path the mutant came from
- `mutations:` each operator with its parameters, in order
- `mode:` `fancy` or `nosmooth`
- `stderr:` the decoder's first stderr line
- `found:` the date, `tools/mutate.py --seed S`, the worker and the case
  number
- `expect:` written when the finding is fixed: `expect: refused CODE` or
  `expect: decoded`

The gate runs the harness over this directory in both modes
(`python3 tools/harness.py corpus/regressions` and the same with
`--nosmooth`) and requires every file to match its note's `expect:` line,
0 failing and no not-yet. The harness skips names ending `.md` and `.txt`,
so this README and the notes are not cases.
