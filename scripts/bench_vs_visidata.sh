#!/usr/bin/env bash
# Benchmark rustidata vs visidata on generated CSVs.
#
# Usage: scripts/bench_vs_visidata.sh [ROWS]
# Requires: vd (visidata) on PATH, python3, a built release binary.

set -euo pipefail
ROWS="${1:-5000000}"
DIR="$(mktemp -d /tmp/rd_bench.XXXX)"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RD="$ROOT/target/release/rustidata"
BENCH_INDEX="$ROOT/target/release/examples/index_bench"
BENCH_SORT="$ROOT/target/release/examples/sort_bench"

echo "== building release binaries =="
cargo build --release -p rustidata -p rustidata-core --examples --manifest-path "$ROOT/Cargo.toml"

echo "== generating $ROWS-row csv in $DIR =="
python3 - "$ROWS" "$DIR/data.csv" <<'EOF'
import sys, random, csv
rows, out = int(sys.argv[1]), sys.argv[2]
random.seed(42)
cities = ['nyc','sf','la','chicago','boston','seattle','austin','denver']
with open(out, 'w', newline='') as f:
    w = csv.writer(f)
    w.writerow(['id','name','city','amount','score','date'])
    for i in range(rows):
        w.writerow([i, f'user{i%10000}', cities[i%8],
                    round(random.uniform(1,10000),2), random.randint(0,100),
                    f'2024-{(i%12)+1:02d}-{(i%28)+1:02d}'])
EOF
ls -lh "$DIR/data.csv"
gzip -kf "$DIR/data.csv"

echo
echo "== rustidata: open + full out-of-core index =="
"$BENCH_INDEX" "$DIR/data.csv"

echo
echo "== rustidata: parallel sort + frequency table =="
"$BENCH_SORT" "$DIR/data.csv" 3

echo
echo "== visidata: batch load (vd -b) =="
/usr/bin/time -p vd -b "$DIR/data.csv" 2>&1 | tail -3 || echo "vd not available"

echo
echo "== rustidata: interactive open->first-frame->quit (wall clock) =="
/usr/bin/time -p bash -c "{ sleep 0.2; printf q; } | script -q /dev/null '$RD' '$DIR/data.csv' > /dev/null" 2>&1 | grep real

echo
echo "results in $DIR (delete when done)"
