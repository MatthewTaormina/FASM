/**
 * Node.js HTTP benchmark server — competitor simulation for FASM Engine.
 *
 * Provides two endpoints mirroring the FASM Engine fixtures:
 *   GET /ping  → 200 {"result":200}
 *   GET /fib   → 200 {"result":832040}  (Fibonacci(30), iterative)
 *
 * Usage:
 *   node node_http_server.js [port]    (default port: 3001)
 *
 * The server writes its PID to stdout on start so the benchmark runner
 * can kill it when done.
 */

'use strict';

const http = require('http');

// Iterative Fibonacci — mirrors fib_handler.fasm exactly
function fib(n) {
    let a = 0, b = 1;
    for (let i = 0; i < n; i++) {
        const tmp = a + b;
        a = b;
        b = tmp;
    }
    return a;
}

const port = parseInt(process.argv[2] || '3001', 10);

const server = http.createServer((req, res) => {
    const url = req.url.split('?')[0];

    if (req.method === 'GET' && url === '/ping') {
        const body = JSON.stringify({ result: 200 });
        res.writeHead(200, {
            'Content-Type': 'application/json',
            'Content-Length': Buffer.byteLength(body),
        });
        res.end(body);
        return;
    }

    if (req.method === 'GET' && url === '/fib') {
        const result = fib(30);
        const body = JSON.stringify({ result });
        res.writeHead(200, {
            'Content-Type': 'application/json',
            'Content-Length': Buffer.byteLength(body),
        });
        res.end(body);
        return;
    }

    res.writeHead(404, { 'Content-Type': 'text/plain' });
    res.end('Not Found');
});

server.listen(port, '127.0.0.1', () => {
    // Signal ready: print PID + port for the benchmark runner
    process.stdout.write(`READY pid=${process.pid} port=${port}\n`);
});

// Graceful shutdown on SIGTERM / SIGINT
function shutdown() {
    server.close(() => process.exit(0));
}
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);
