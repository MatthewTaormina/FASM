/**
 * native_rust_bench.js — throughput harness that measures a single HTTP endpoint
 * using Node.js http.request (no external deps) and reports req/s.
 *
 * Used internally by run_benchmarks.sh to benchmark each competitor server.
 *
 * Usage:
 *   node http_bench_client.js <url> <concurrency> <total_requests>
 *
 * Output (one line to stdout):
 *   RESULT url=<url> concurrency=<n> total=<n> rps=<n.n> p50_ms=<n.n> p99_ms=<n.n> errors=<n>
 */
'use strict';

const http = require('http');
const { URL } = require('url');

const [,, urlArg, concArg, totalArg] = process.argv;
if (!urlArg || !concArg || !totalArg) {
    process.stderr.write('Usage: node http_bench_client.js <url> <concurrency> <total_requests>\n');
    process.exit(1);
}

const TARGET_URL = new URL(urlArg);
const CONCURRENCY = parseInt(concArg, 10);
const TOTAL = parseInt(totalArg, 10);

const latencies = [];
let errors = 0;
let completed = 0;

function doRequest(resolve) {
    const t0 = process.hrtime.bigint();
    const req = http.request({
        hostname: TARGET_URL.hostname,
        port: TARGET_URL.port || 80,
        path: TARGET_URL.pathname,
        method: 'GET',
        agent: false,
    }, (res) => {
        res.resume(); // drain
        res.on('end', () => {
            const ms = Number(process.hrtime.bigint() - t0) / 1e6;
            if (res.statusCode === 200) {
                latencies.push(ms);
            } else {
                errors++;
            }
            resolve();
        });
    });
    req.on('error', () => { errors++; resolve(); });
    req.end();
}

async function runWorker() {
    while (true) {
        const myIdx = completed++;
        if (myIdx >= TOTAL) break;
        await new Promise(resolve => doRequest(resolve));
    }
}

(async () => {
    const wallStart = process.hrtime.bigint();

    await Promise.all(Array.from({ length: CONCURRENCY }, () => runWorker()));

    const wallMs = Number(process.hrtime.bigint() - wallStart) / 1e6;
    const rps = (latencies.length / wallMs) * 1000;

    latencies.sort((a, b) => a - b);
    const p50 = latencies[Math.floor(latencies.length * 0.50)] || 0;
    const p99 = latencies[Math.floor(latencies.length * 0.99)] || 0;

    process.stdout.write(
        `RESULT url=${urlArg} concurrency=${CONCURRENCY} total=${TOTAL} ` +
        `rps=${rps.toFixed(1)} p50_ms=${p50.toFixed(2)} p99_ms=${p99.toFixed(2)} errors=${errors}\n`
    );
})();
