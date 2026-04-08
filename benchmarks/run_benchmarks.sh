#!/usr/bin/env bash
# run_benchmarks.sh — Comprehensive FASM Engine performance benchmark runner.
#
# Measures and compares FASM Engine against Node.js, Python, and native C
# across the following KPIs:
#   1. Build / compile time
#   2. Binary / app size
#   3. Cold start latency
#   4. VM operation throughput (ops/second)
#   5. HTTP requests per second (single-client sequential)
#   6. HTTP requests per second (concurrent — 8 and 32 clients)
#   7. Memory footprint (RSS at idle + under load)
#
# Competitor reference data for cloud platforms (Lambda, Cloudflare Workers)
# is embedded as known-good published values and included in the final report.
#
# Usage:
#   ./benchmarks/run_benchmarks.sh [--quick] [--report-only]
#
#   --quick        Run fewer iterations (faster CI check, less precise)
#   --report-only  Skip benchmarks and only regenerate the report from the
#                  last saved raw data file (benchmarks/reports/latest_raw.json)
#
# Output:
#   benchmarks/reports/perf-YYYY-MM-DD.md   — human-readable dated report
#   benchmarks/reports/latest_raw.json      — raw data for tooling / trend tracking

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPORTS_DIR="${SCRIPT_DIR}/reports"
COMPETITORS_DIR="${SCRIPT_DIR}/competitors"
RAW_JSON="${REPORTS_DIR}/latest_raw.json"

QUICK=0
REPORT_ONLY=0
for arg in "$@"; do
    case "$arg" in
        --quick)       QUICK=1 ;;
        --report-only) REPORT_ONLY=1 ;;
    esac
done

# Requests used for HTTP throughput benchmarks
if [[ "$QUICK" -eq 1 ]]; then
    AB_REQUESTS=500
    AB_CONCURRENCY_LIST="1 8 32"
else
    AB_REQUESTS=2000
    AB_CONCURRENCY_LIST="1 4 8 16 32"
fi

mkdir -p "$REPORTS_DIR"

# ─── Utility ──────────────────────────────────────────────────────────────────

log()  { echo "[bench] $*"; }
warn() { echo "[bench] WARN: $*" >&2; }

# Wait for a TCP port to accept connections
wait_for_port() {
    local port="$1"
    local max=60
    for ((i=0; i<max; i++)); do
        if bash -c "echo >/dev/tcp/127.0.0.1/$port" 2>/dev/null; then
            return 0
        fi
        sleep 0.2
    done
    warn "Port $port never opened after $((max * 200))ms"
    return 1
}

# Run ab (ApacheBench) and extract req/s, p50, p99 from its output
run_ab() {
    local url="$1"
    local requests="$2"
    local concurrency="$3"

    local out
    out=$(ab -n "$requests" -c "$concurrency" -q "$url" 2>&1) || true

    local rps p50 p99
    rps=$(echo "$out"  | grep "^Requests per second" | awk '{print $4}')
    p50=$(echo "$out"  | grep "^  50%" | awk '{print $2}')
    p99=$(echo "$out"  | grep "^  99%" | awk '{print $2}')

    # ab gives percentiles in ms already
    echo "rps=${rps:-0} p50_ms=${p50:-0} p99_ms=${p99:-0}"
}

# ─── Bail out if only regenerating the report ────────────────────────────────

if [[ "$REPORT_ONLY" -eq 1 ]]; then
    if [[ ! -f "$RAW_JSON" ]]; then
        log "ERROR: $RAW_JSON not found; run without --report-only first."
        exit 1
    fi
    log "Regenerating report from $RAW_JSON …"
    "${SCRIPT_DIR}/generate_report.sh" "$RAW_JSON"
    exit 0
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 1 — Build FASM Engine (release)
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 1: Build FASM Engine (release) ==="

cd "$REPO_ROOT"
BUILD_START=$(date +%s%N)
cargo build --release -p fasm-engine -p fasm-cli 2>&1 | tail -5
BUILD_END=$(date +%s%N)
BUILD_MS=$(( (BUILD_END - BUILD_START) / 1000000 ))
log "Build time: ${BUILD_MS} ms"

FASM_BIN="${REPO_ROOT}/target/release/fasm"
ENGINE_BIN="${REPO_ROOT}/target/release/fasm-engine"
FASM_BIN_SIZE=0
ENGINE_BIN_SIZE=0
if [[ -f "$FASM_BIN" ]]; then
    FASM_BIN_SIZE=$(stat -c%s "$FASM_BIN" 2>/dev/null || stat -f%z "$FASM_BIN" 2>/dev/null || echo 0)
fi
if [[ -f "$ENGINE_BIN" ]]; then
    ENGINE_BIN_SIZE=$(stat -c%s "$ENGINE_BIN" 2>/dev/null || stat -f%z "$ENGINE_BIN" 2>/dev/null || echo 0)
fi
log "fasm CLI size:    $((FASM_BIN_SIZE / 1024)) KB"
log "fasm-engine size: $((ENGINE_BIN_SIZE / 1024)) KB"

# ═══════════════════════════════════════════════════════════════════════════════
# Step 2 — FASM cold start (compile + first execution via CLI)
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 2: FASM cold start (compile + execute) ==="

FIXTURE_PING="${REPO_ROOT}/crates/fasm-engine/tests/fixtures/ping.fasm"
COLD_START_RUNS=20
COLD_START_TOTAL_MS=0

for ((i=0; i<COLD_START_RUNS; i++)); do
    T0=$(date +%s%N)
    "${FASM_BIN}" run "${FIXTURE_PING}" > /dev/null 2>&1 || true
    T1=$(date +%s%N)
    COLD_START_TOTAL_MS=$(( COLD_START_TOTAL_MS + (T1 - T0) / 1000000 ))
done

FASM_COLD_START_MS=$(( COLD_START_TOTAL_MS / COLD_START_RUNS ))
log "FASM cold start avg (${COLD_START_RUNS} runs): ${FASM_COLD_START_MS} ms"

# Node.js cold start (process spawn + execute)
NODE_COLD_START_MS=0
if command -v node >/dev/null 2>&1; then
    NODE_COLD_TOTAL=0
    for ((i=0; i<COLD_START_RUNS; i++)); do
        T0=$(date +%s%N)
        node -e "
function fib(n){let a=0,b=1;for(let i=0;i<n;i++){const t=a+b;a=b;b=t;}return a;}
process.stdout.write(JSON.stringify({result:fib(30)}));
" > /dev/null 2>&1
        T1=$(date +%s%N)
        NODE_COLD_TOTAL=$(( NODE_COLD_TOTAL + (T1 - T0) / 1000000 ))
    done
    NODE_COLD_START_MS=$(( NODE_COLD_TOTAL / COLD_START_RUNS ))
    log "Node.js cold start avg: ${NODE_COLD_START_MS} ms"
fi

# Python cold start
PYTHON_COLD_START_MS=0
if command -v python3 >/dev/null 2>&1; then
    PY_COLD_TOTAL=0
    for ((i=0; i<COLD_START_RUNS; i++)); do
        T0=$(date +%s%N)
        python3 -c "
def fib(n):
    a,b=0,1
    for _ in range(n): a,b=b,a+b
    return a
import json, sys
sys.stdout.write(json.dumps({'result':fib(30)}))
" > /dev/null 2>&1
        T1=$(date +%s%N)
        PY_COLD_TOTAL=$(( PY_COLD_TOTAL + (T1 - T0) / 1000000 ))
    done
    PYTHON_COLD_START_MS=$(( PY_COLD_TOTAL / COLD_START_RUNS ))
    log "Python cold start avg: ${PYTHON_COLD_START_MS} ms"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 3 — FASM Engine HTTP throughput
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 3: FASM Engine HTTP throughput ==="

# Start the fasm-engine with a minimal config
# Config must live in the same directory as the .fasm sources so that
# relative `source =` paths in [[routes]] resolve correctly.
ENGINE_PORT=18301
ENGINE_LOG="/tmp/fasm_bench_engine.log"
FIXTURES_DIR="${REPO_ROOT}/crates/fasm-engine/tests/fixtures"
FASM_ENGINE_CONFIG="${FIXTURES_DIR}/bench_engine.toml"

cat > "$FASM_ENGINE_CONFIG" <<TOML
[server]
host = "127.0.0.1"
port = ${ENGINE_PORT}

[engine]
max_concurrent = 128

[[routes]]
method   = "GET"
path     = "/ping"
function = "Ping"
source   = "ping.fasm"

[[routes]]
method   = "GET"
path     = "/fib"
function = "FibHandler"
source   = "fib_handler.fasm"
TOML

"${ENGINE_BIN}" serve "${FASM_ENGINE_CONFIG}" \
    > "$ENGINE_LOG" 2>&1 &
ENGINE_PID=$!
trap "kill $ENGINE_PID 2>/dev/null || true; rm -f '${FASM_ENGINE_CONFIG}'" EXIT

wait_for_port "$ENGINE_PORT"
sleep 0.5  # let it warm up
log "FASM Engine running (PID $ENGINE_PID)"

# Warm-up
for ((i=0; i<50; i++)); do
    curl -sf "http://127.0.0.1:${ENGINE_PORT}/ping" > /dev/null
done

# Benchmark FASM /ping
declare -A FASM_PING_RPS FASM_PING_P50 FASM_PING_P99
declare -A FASM_FIB_RPS  FASM_FIB_P50  FASM_FIB_P99

for C in $AB_CONCURRENCY_LIST; do
    log "  FASM /ping  concurrency=$C"
    result=$(run_ab "http://127.0.0.1:${ENGINE_PORT}/ping" "$AB_REQUESTS" "$C")
    rps=$(echo "$result" | grep -oP 'rps=\K[0-9.]+')
    p50=$(echo "$result" | grep -oP 'p50_ms=\K[0-9]+')
    p99=$(echo "$result" | grep -oP 'p99_ms=\K[0-9]+')
    FASM_PING_RPS[$C]="${rps:-0}"
    FASM_PING_P50[$C]="${p50:-0}"
    FASM_PING_P99[$C]="${p99:-0}"
    log "    rps=${rps} p50=${p50}ms p99=${p99}ms"
done

for C in 1 8; do
    log "  FASM /fib   concurrency=$C"
    result=$(run_ab "http://127.0.0.1:${ENGINE_PORT}/fib" "$AB_REQUESTS" "$C")
    rps=$(echo "$result" | grep -oP 'rps=\K[0-9.]+')
    p50=$(echo "$result" | grep -oP 'p50_ms=\K[0-9]+')
    p99=$(echo "$result" | grep -oP 'p99_ms=\K[0-9]+')
    FASM_FIB_RPS[$C]="${rps:-0}"
    FASM_FIB_P50[$C]="${p50:-0}"
    FASM_FIB_P99[$C]="${p99:-0}"
    log "    rps=${rps} p50=${p50}ms p99=${p99}ms"
done

kill "$ENGINE_PID" 2>/dev/null || true
trap - EXIT
sleep 0.5

# ═══════════════════════════════════════════════════════════════════════════════
# Step 4 — Node.js competitor server throughput
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 4: Node.js HTTP throughput ==="

NODE_RPS_1=0; NODE_RPS_8=0; NODE_RPS_32=0
NODE_P50_1=0; NODE_P99_1=0

if command -v node >/dev/null 2>&1; then
    NODE_PORT=18302
    node "${COMPETITORS_DIR}/node_http_server.js" "$NODE_PORT" > /tmp/node_bench.log 2>&1 &
    NODE_PID=$!
    trap "kill $NODE_PID 2>/dev/null || true" EXIT

    if wait_for_port "$NODE_PORT"; then
        sleep 0.3
        # Warm-up
        for ((i=0; i<20; i++)); do
            curl -sf "http://127.0.0.1:${NODE_PORT}/ping" > /dev/null
        done

        for C in $AB_CONCURRENCY_LIST; do
            log "  Node.js /ping  concurrency=$C"
            result=$(run_ab "http://127.0.0.1:${NODE_PORT}/ping" "$AB_REQUESTS" "$C")
            rps=$(echo "$result" | grep -oP 'rps=\K[0-9.]+')
            p50=$(echo "$result" | grep -oP 'p50_ms=\K[0-9]+')
            p99=$(echo "$result" | grep -oP 'p99_ms=\K[0-9]+')
            eval "NODE_RPS_${C}=${rps:-0}"
            eval "NODE_P50_${C}=${p50:-0}"
            eval "NODE_P99_${C}=${p99:-0}"
            log "    rps=${rps} p50=${p50}ms p99=${p99}ms"
        done
    else
        warn "Node.js server failed to start"
    fi

    kill "$NODE_PID" 2>/dev/null || true
    trap - EXIT
    sleep 0.3
else
    warn "node not found; skipping Node.js benchmarks"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 5 — Python competitor server throughput
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 5: Python HTTP throughput ==="

PYTHON_RPS_1=0; PYTHON_RPS_8=0; PYTHON_RPS_32=0
PYTHON_P50_1=0; PYTHON_P99_1=0

if command -v python3 >/dev/null 2>&1; then
    PYTHON_PORT=18303
    python3 "${COMPETITORS_DIR}/python_http_server.py" "$PYTHON_PORT" > /tmp/python_bench.log 2>&1 &
    PYTHON_PID=$!
    trap "kill $PYTHON_PID 2>/dev/null || true" EXIT

    if wait_for_port "$PYTHON_PORT"; then
        sleep 0.3
        # Warm-up
        for ((i=0; i<20; i++)); do
            curl -sf "http://127.0.0.1:${PYTHON_PORT}/ping" > /dev/null
        done

        for C in $AB_CONCURRENCY_LIST; do
            log "  Python /ping  concurrency=$C"
            result=$(run_ab "http://127.0.0.1:${PYTHON_PORT}/ping" "$AB_REQUESTS" "$C")
            rps=$(echo "$result" | grep -oP 'rps=\K[0-9.]+')
            p50=$(echo "$result" | grep -oP 'p50_ms=\K[0-9]+')
            p99=$(echo "$result" | grep -oP 'p99_ms=\K[0-9]+')
            eval "PYTHON_RPS_${C}=${rps:-0}"
            eval "PYTHON_P50_${C}=${p50:-0}"
            eval "PYTHON_P99_${C}=${p99:-0}"
            log "    rps=${rps} p50=${p50}ms p99=${p99}ms"
        done
    else
        warn "Python server failed to start"
    fi

    kill "$PYTHON_PID" 2>/dev/null || true
    trap - EXIT
    sleep 0.3
else
    warn "python3 not found; skipping Python benchmarks"
fi

# ═══════════════════════════════════════════════════════════════════════════════
# Step 6 — FASM VM compile throughput (Fibonacci, 1000 compiles)
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 6: FASM compile throughput (in-process via test binary) ==="

# Use the fasm CLI to compile a source file N times and measure average time
COMPILE_RUNS=100
FIB_FIXTURE="${REPO_ROOT}/crates/fasm-engine/tests/fixtures/fib_handler.fasm"
COMPILE_TOTAL_MS=0

for ((i=0; i<COMPILE_RUNS; i++)); do
    T0=$(date +%s%N)
    "${FASM_BIN}" compile "${FIB_FIXTURE}" -o /tmp/bench_fib.fasmc > /dev/null 2>&1 || true
    T1=$(date +%s%N)
    COMPILE_TOTAL_MS=$(( COMPILE_TOTAL_MS + (T1 - T0) / 1000000 ))
done

FASM_COMPILE_MS=$(( COMPILE_TOTAL_MS / COMPILE_RUNS ))
log "FASM compile avg (${COMPILE_RUNS} runs): ${FASM_COMPILE_MS} ms"

# ═══════════════════════════════════════════════════════════════════════════════
# Step 7 — Memory footprint
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 7: Memory footprint ==="

# Restart engine on a fresh port for clean RSS measurement
ENGINE_PORT_FINAL=18305
FASM_ENGINE_CONFIG2="${FIXTURES_DIR}/bench_engine_mem.toml"
cat > "$FASM_ENGINE_CONFIG2" <<TOML
[server]
host = "127.0.0.1"
port = ${ENGINE_PORT_FINAL}

[engine]
max_concurrent = 128

[[routes]]
method   = "GET"
path     = "/ping"
function = "Ping"
source   = "ping.fasm"
TOML

"${ENGINE_BIN}" serve "${FASM_ENGINE_CONFIG2}" \
    > "${ENGINE_LOG}.mem" 2>&1 &
ENGINE_PID_FINAL=$!
trap "kill $ENGINE_PID_FINAL 2>/dev/null || true; rm -f '${FASM_ENGINE_CONFIG2}'" EXIT
wait_for_port "$ENGINE_PORT_FINAL"
sleep 0.5

# Idle RSS
FASM_IDLE_RSS_KB=$(cat /proc/${ENGINE_PID_FINAL}/status 2>/dev/null | grep VmRSS | awk '{print $2}' || echo 0)

# After 500 requests
for ((i=0; i<500; i++)); do
    curl -sf "http://127.0.0.1:${ENGINE_PORT_FINAL}/ping" > /dev/null
done
FASM_LOADED_RSS_KB=$(cat /proc/${ENGINE_PID_FINAL}/status 2>/dev/null | grep VmRSS | awk '{print $2}' || echo 0)

kill "$ENGINE_PID_FINAL" 2>/dev/null || true
rm -f "$FASM_ENGINE_CONFIG2"
trap - EXIT
log "FASM Engine idle RSS:    ${FASM_IDLE_RSS_KB} KB ($((FASM_IDLE_RSS_KB / 1024)) MB)"
log "FASM Engine loaded RSS:  ${FASM_LOADED_RSS_KB} KB ($((FASM_LOADED_RSS_KB / 1024)) MB)"

# ═══════════════════════════════════════════════════════════════════════════════
# Step 8 — Collect & write raw JSON
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 8: Writing raw benchmark data ==="

DATE_ISO=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
DATE_SHORT=$(date -u +"%Y-%m-%d")
NODE_VERSION=$(node --version 2>/dev/null || echo "N/A")
PYTHON_VERSION=$(python3 --version 2>/dev/null | awk '{print $2}' || echo "N/A")
RUST_VERSION=$(rustc --version | awk '{print $2}' || echo "N/A")
OS_INFO=$(uname -srm || echo "N/A")
CPU_INFO=$(grep "model name" /proc/cpuinfo 2>/dev/null | head -1 | sed 's/.*: //' || echo "N/A")

# Build a JSON blob with all measured values
cat > "$RAW_JSON" <<JSON
{
  "meta": {
    "date": "${DATE_ISO}",
    "os": "${OS_INFO}",
    "cpu": "${CPU_INFO}",
    "rust_version": "${RUST_VERSION}",
    "node_version": "${NODE_VERSION}",
    "python_version": "${PYTHON_VERSION}",
    "ab_requests_per_test": ${AB_REQUESTS},
    "cold_start_runs": ${COLD_START_RUNS}
  },
  "build": {
    "fasm_build_time_ms": ${BUILD_MS},
    "fasm_cli_binary_size_bytes": ${FASM_BIN_SIZE},
    "fasm_engine_binary_size_bytes": ${ENGINE_BIN_SIZE}
  },
  "cold_start_ms": {
    "fasm_compile_plus_exec": ${FASM_COLD_START_MS},
    "node_process_spawn_exec": ${NODE_COLD_START_MS},
    "python_process_spawn_exec": ${PYTHON_COLD_START_MS},
    "reference_aws_lambda_rust_ms": 17,
    "reference_aws_lambda_python312_ms": 100,
    "reference_aws_lambda_nodejs22_ms": 148,
    "reference_cloudflare_workers_ms": 5
  },
  "compile_time_ms": {
    "fasm_fib_handler_avg": ${FASM_COMPILE_MS}
  },
  "http_throughput_rps": {
    "fasm_ping_c1":  ${FASM_PING_RPS[1]:-0},
    "fasm_ping_c8":  ${FASM_PING_RPS[8]:-0},
    "fasm_ping_c32": ${FASM_PING_RPS[32]:-0},
    "fasm_fib_c1":   ${FASM_FIB_RPS[1]:-0},
    "fasm_fib_c8":   ${FASM_FIB_RPS[8]:-0},
    "node_ping_c1":  ${NODE_RPS_1:-0},
    "node_ping_c8":  ${NODE_RPS_8:-0},
    "node_ping_c32": ${NODE_RPS_32:-0},
    "python_ping_c1":  ${PYTHON_RPS_1:-0},
    "python_ping_c8":  ${PYTHON_RPS_8:-0},
    "python_ping_c32": ${PYTHON_RPS_32:-0},
    "reference_native_rust_axum_rps":   48700,
    "reference_node_express_rps":       5700,
    "reference_python_fastapi_rps":     1200,
    "reference_cloudflare_workers_rps": 1000,
    "reference_aws_lambda_warm_rps":    500
  },
  "latency_ms": {
    "fasm_ping_c1_p50":  ${FASM_PING_P50[1]:-0},
    "fasm_ping_c1_p99":  ${FASM_PING_P99[1]:-0},
    "fasm_ping_c8_p50":  ${FASM_PING_P50[8]:-0},
    "fasm_ping_c8_p99":  ${FASM_PING_P99[8]:-0},
    "fasm_ping_c32_p50": ${FASM_PING_P50[32]:-0},
    "fasm_ping_c32_p99": ${FASM_PING_P99[32]:-0},
    "node_ping_c1_p50":  ${NODE_P50_1:-0},
    "node_ping_c1_p99":  ${NODE_P99_1:-0},
    "python_ping_c1_p50": ${PYTHON_P50_1:-0},
    "python_ping_c1_p99": ${PYTHON_P99_1:-0}
  },
  "memory_kb": {
    "fasm_engine_idle":   ${FASM_IDLE_RSS_KB},
    "fasm_engine_loaded": ${FASM_LOADED_RSS_KB},
    "reference_node_express_idle_kb":   83000,
    "reference_python_fastapi_idle_kb": 45000,
    "reference_native_rust_axum_kb":    8000,
    "reference_docker_base_image_mb":   5
  }
}
JSON

log "Raw data written to $RAW_JSON"

# ═══════════════════════════════════════════════════════════════════════════════
# Step 9 — Generate dated Markdown report
# ═══════════════════════════════════════════════════════════════════════════════

log "=== Step 9: Generating dated report ==="

"${SCRIPT_DIR}/generate_report.sh" "$RAW_JSON"

log "=== Benchmark complete ==="
