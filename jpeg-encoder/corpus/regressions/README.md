# regressions/

Every input that made `clean` fail a class of the clean harness (mismatch,
trap, hang, cpu, memory, odd, or `disagree`: a refusal agreeing with
neither the decoder nor the metadata oracle) is saved here by
tools/mutate_clean.py as
`<source stem>-<mutation kinds joined by '-'>-<first 8 hex digits of the input's SHA-1>.jpg`,
with a note of the same name ending `.txt`. The note's keys, one
`key: value` a line, in this order:

- `class:` the fuzzer's class
- `code:` the refusal code, or `-`
- `source:` the source the mutant came from, its path under the Whackford
  directory
- `mutations:` the operators' notes, joined by `; `
- `quality:` and `sample:` the setting clean ran at (`sample: grey` for a
  greyscale decode)
- `kind:` `grey` when the decoder's decode is P5, `colour` otherwise
- `decoder:` the decoder harness's class and code for the input
- `oracle:` the metadata oracle's prediction (`META O B`, `REFUSED CODE`
  or `UNPARSABLE`)
- `stderr:` clean's first stderr line
- `note:` the clean harness's message (the mismatch byte and segment, the
  odd reason, `decoder: ..., oracle: ...` for a disagreement)
- `found:` the date and the fuzzer's seed, worker and case

The fuzzer always runs clean with fancy upsampling, the decoder's default,
so there is no `mode:` key. When the finding is fixed, the note gets
`expect: match <orientation> <icc-bytes>` (the fixed clean's CLEANED fields,
which the harness checks against the oracle) or `expect: refused <CODE>`.
A finding whose `decoder:` line is itself a failing class is a decoder
finding and never gets an `expect:` line here: the decoder is fixed
separately.

The gate builds a manifest from the notes, one line a note
(`<name>.jpg <kind> <expect's value>`), with this one line:

    python3 -I -c "import glob,os; w=open('../tmp/e7/reg-manifest.txt','w'); [w.write(os.path.basename(n)[:-4]+'.jpg '+(lambda d: d.get('kind','colour')+' '+d['expect'])(dict(l.split(': ',1) for l in open(n).read().splitlines() if ': ' in l))+'\n') for n in sorted(glob.glob('corpus/regressions/*.txt'))]; w.close()"

(a note without `expect:` makes it fail, which is the point), then runs

    python3 -I tools/harness.py clean --quality 40,85 --jobs 38 --manifest ../tmp/e7/reg-manifest.txt --out ../tmp/e7/reg corpus/regressions

which requires every case of every regression to match its line, with 0
failing. The harness skips names ending .md and .txt, so this README and
the notes are not cases.
