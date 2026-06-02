#!/usr/bin/env bash
#
# check-perf-regression.sh — developer/CI timing regression gate.
#
# Runs the full criterion bench suite, parses the FRESH median point estimates
# that criterion writes under target/criterion/<id>/new/estimates.json, maps
# each bench id to a metric key in bench/baselines.json, and fails (non-zero
# exit) if any metric regressed beyond the bench_regression_threshold_percent
# tolerance recorded in baselines.json.
#
# This is the slow, real-timing counterpart to the deterministic Rust guard in
# crates/mimir-core/tests/perf_regression.rs (which only checks the committed
# baseline against its targets and never runs a bench).
#
# Usage:
#   scripts/check-perf-regression.sh            # run benches, then compare
#   SKIP_BENCH=1 scripts/check-perf-regression.sh   # reuse existing criterion data
#
# Requires: cargo (1.95), python3. No network or provider calls — every bench
# uses local synthetic in-repo data.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINES="${REPO_ROOT}/bench/baselines.json"
CRITERION_DIR="${REPO_ROOT}/target/criterion"

if [[ ! -f "${BASELINES}" ]]; then
  echo "error: ${BASELINES} not found" >&2
  exit 2
fi

# Ensure cargo is on PATH (the repo convention sources ~/.cargo/env).
if ! command -v cargo >/dev/null 2>&1; then
  # shellcheck disable=SC1090
  [[ -f "${HOME}/.cargo/env" ]] && . "${HOME}/.cargo/env"
fi

# Crates that own the tracked benches.
BENCH_CRATES=(mimir-providers mimir-context mimir-index mimir-tui mimir-session)

if [[ "${SKIP_BENCH:-0}" != "1" ]]; then
  echo "==> Running benches for: ${BENCH_CRATES[*]}"
  for crate in "${BENCH_CRATES[@]}"; do
    echo "--- cargo bench -p ${crate} ---"
    ( cd "${REPO_ROOT}" && cargo bench -p "${crate}" )
  done
else
  echo "==> SKIP_BENCH=1: reusing existing criterion estimates in ${CRITERION_DIR}"
fi

echo "==> Comparing fresh criterion medians against ${BASELINES}"

# All comparison + parsing logic lives in python3 for robust JSON handling.
CRITERION_DIR="${CRITERION_DIR}" BASELINES="${BASELINES}" python3 - <<'PY'
import json
import os
import sys

criterion_dir = os.environ["CRITERION_DIR"]
baselines_path = os.environ["BASELINES"]

with open(baselines_path) as fh:
    base = json.load(fh)

metrics = base.get("metrics", {})
threshold_pct = float(base.get("ci_gates", {}).get("bench_regression_threshold_percent", 20))

# metric_key -> (criterion bench id, unit divisor from ns to the metric unit)
NS_PER_MS = 1_000_000.0
NS_PER_S = 1_000_000_000.0
MAPPING = {
    "mimir_init_ms":               ("session_init/init_project_files",      NS_PER_MS),
    "cold_startup_warm_cache_ms":  ("context_packet_build",                 NS_PER_MS),
    "cold_startup_no_cache_ms":    ("context_packet_build",                 NS_PER_MS),
    "packet_build_small_ms":       ("context_packet_build",                 NS_PER_MS),
    "repo_index_10k_cold_s":       ("repo_index/cold_build_index_600",      NS_PER_S),
    "repo_index_incremental_s":    ("repo_index/incremental_reindex_600",   NS_PER_S),
    "token_count_local_ms":        ("count_local/102420",                   NS_PER_MS),
    "tui_render_frame_ms":         ("render_frame_120x40",                  NS_PER_MS),
    "doctor_command_s":            ("session_doctor/doctor_probe",          NS_PER_S),
}

def read_median_ns(bench_id):
    path = os.path.join(criterion_dir, bench_id, "new", "estimates.json")
    if not os.path.exists(path):
        return None
    with open(path) as fh:
        est = json.load(fh)
    return est["median"]["point_estimate"]

regressions = []
missing = []
rows = []

for metric, (bench_id, divisor) in MAPPING.items():
    baseline = metrics.get(metric)
    if baseline is None:
        missing.append(f"metric `{metric}` absent from baselines.json")
        continue
    median_ns = read_median_ns(bench_id)
    if median_ns is None:
        missing.append(f"no fresh criterion estimate for `{bench_id}` (metric `{metric}`)")
        continue
    fresh = median_ns / divisor
    # Allowed ceiling = baseline * (1 + threshold/100).
    allowed = float(baseline) * (1.0 + threshold_pct / 100.0)
    delta_pct = ((fresh - float(baseline)) / float(baseline) * 100.0) if baseline else float("inf")
    status = "OK"
    if fresh > allowed:
        status = "REGRESSION"
        regressions.append((metric, baseline, fresh, delta_pct))
    rows.append((metric, bench_id, float(baseline), fresh, delta_pct, status))

w = max((len(r[0]) for r in rows), default=10)
print(f"\n  Regression threshold: +{threshold_pct:.0f}% over baseline\n")
print(f"  {'metric'.ljust(w)}  {'baseline':>12}  {'fresh':>12}  {'delta':>9}  status")
print(f"  {'-'*w}  {'-'*12}  {'-'*12}  {'-'*9}  ------")
for metric, bench_id, baseline, fresh, delta_pct, status in rows:
    print(f"  {metric.ljust(w)}  {baseline:12.6f}  {fresh:12.6f}  {delta_pct:+8.2f}%  {status}")

if missing:
    print("\nERROR: incomplete data:", file=sys.stderr)
    for m in missing:
        print(f"  - {m}", file=sys.stderr)
    sys.exit(2)

if regressions:
    print("\nFAIL: performance regression detected:", file=sys.stderr)
    for metric, baseline, fresh, delta_pct in regressions:
        print(f"  - {metric}: baseline {baseline} -> {fresh:.6f} ({delta_pct:+.2f}%)", file=sys.stderr)
    print(
        "\nInvestigate the hot path or, if the new number is intentional, "
        "re-run the bench harness to refresh bench/baselines.json and file an "
        "ADR per 17-PERFORMANCE-TARGETS.md.",
        file=sys.stderr,
    )
    sys.exit(1)

print("\nPASS: no metric regressed beyond tolerance.")
PY
