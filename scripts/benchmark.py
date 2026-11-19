#!/usr/bin/env python3
"""Reproducible CLI benchmark: measures whole-process latency (not parser-only)."""
import argparse
from pathlib import Path
import statistics
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--binary', default='engine/target/release/sql-lineage-rs')
parser.add_argument('--runs', type=int, default=5)
args = parser.parse_args()
if args.runs < 1:
    raise SystemExit('--runs must be >= 1')
if not Path(args.binary).is_file():
    raise SystemExit('Compile first: cargo build --release --manifest-path engine/Cargo.toml')

for count in (10, 50, 100, 250, 500):
    sql = ''.join(f'CREATE VIEW reporting.v{i} AS SELECT r.id FROM raw.orders r;\n' for i in range(count))
    timings = []
    for _ in range(args.runs):
        start = time.perf_counter()
        run = subprocess.run([args.binary, '--format', 'json', '-'],
                             input=sql, text=True, capture_output=True, timeout=25)
        if run.returncode:
            raise SystemExit(run.stderr)
        timings.append((time.perf_counter()-start)*1000)
    print(f'{count:4} statements | median={statistics.median(timings):8.2f} ms | min={min(timings):8.2f} ms')
