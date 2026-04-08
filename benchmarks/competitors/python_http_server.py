"""
Python HTTP benchmark server — competitor simulation for FASM Engine.

Provides two endpoints mirroring the FASM Engine fixtures:
  GET /ping  → 200  {"result": 200}
  GET /fib   → 200  {"result": 832040}   (Fibonacci(30), iterative)

Uses only the standard library (http.server) to avoid dependency issues.

Usage:
  python3 python_http_server.py [port]   (default port: 3002)
"""
import http.server
import json
import os
import signal
import sys
import threading


def fib(n: int) -> int:
    """Iterative Fibonacci — mirrors fib_handler.fasm."""
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a


PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 3002


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):  # silence access log
        pass

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/ping":
            body = json.dumps({"result": 200}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif path == "/fib":
            body = json.dumps({"result": fib(30)}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"Not Found")


server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)


def shutdown(signum, frame):
    threading.Thread(target=server.shutdown).start()


signal.signal(signal.SIGTERM, shutdown)
signal.signal(signal.SIGINT, shutdown)

# Signal ready to the benchmark runner
sys.stdout.write(f"READY pid={os.getpid()} port={PORT}\n")
sys.stdout.flush()

server.serve_forever()
