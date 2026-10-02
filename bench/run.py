"""Run-speed benchmark: Chasm against the same algorithms in Rust and JavaScript.

Each task is a Chasm example plus a file in bench/ that replaces its `main`,
with ports in bench/rust/ and bench/js/. Every program prints a checksum and
the nanoseconds its work took, timed inside the program, so start-up and
compilation are not counted. The checksums must agree.

Usage: python3 bench/run.py [--runs N] [--record]
  --record  append the results, dated, to docs/performance.md
"""

import argparse
import datetime
import pathlib
import platform
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TASKS = ["sieve", "mandelbrot", "n-queens", "quicksort"]
OUT = ROOT / "tmp" / "bench"


def sh(cmd, **kw):
    return subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True, **kw).stdout


def build():
    OUT.mkdir(parents=True, exist_ok=True)
    sh(["cargo", "build", "-q", "--release", "-p", "chasm-cli"])
    for t in TASKS:
        sh(["rustc", "-C", "opt-level=3", "-o", str(OUT / t), f"bench/rust/{t}.rs"])


def commands(t):
    return {
        "chasm": ["target/release/chasm", "run", f"examples/{t}.chasm", f"bench/{t}.chasm"],
        "rust": [str(OUT / t)],
        "js": ["node", f"bench/js/{t}.mjs"],
    }


def measure(cmd, runs):
    checks, times = set(), []
    for _ in range(runs):
        check, ns = sh(cmd).split()
        checks.add(check)
        times.append(int(ns) / 1e6)
    if len(checks) != 1:
        sys.exit(f"{cmd}: checksum changed between runs: {checks}")
    return checks.pop(), statistics.median(times)


def versions():
    cpu = platform.processor() or platform.machine()
    try:
        for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    rustc = sh(["rustc", "--version"]).strip()
    node = sh(["node", "--version"]).strip()
    commit = sh(["git", "rev-parse", "--short", "HEAD"]).strip()
    return f"commit {commit}; {rustc}; node {node}; {cpu}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--record", action="store_true")
    args = ap.parse_args()
    build()
    rows = []
    for t in TASKS:
        res = {lang: measure(cmd, args.runs) for lang, cmd in commands(t).items()}
        checks = {c for c, _ in res.values()}
        if len(checks) != 1:
            sys.exit(f"{t}: checksums disagree: {res}")
        c, r, j = (res[k][1] for k in ("chasm", "rust", "js"))
        rows.append(f"| {t} | {c:.1f} | {r:.1f} | {j:.1f} | {c / r:.2f} | {c / j:.2f} |")
    table = "\n".join(
        [
            "| Task | Chasm ms | Rust ms | JS ms | Chasm / Rust | Chasm / JS |",
            "|---|---:|---:|---:|---:|---:|",
            *rows,
        ]
    )
    print(table)
    if args.record:
        today = datetime.date.today().isoformat()
        section = f"\n### {today}\n\n{versions()}; median of {args.runs} runs.\n\n{table}\n"
        with open(ROOT / "docs" / "performance.md", "a") as f:
            f.write(section)
        print("recorded in docs/performance.md", file=sys.stderr)


if __name__ == "__main__":
    main()
