import time

def fib(n):
    if n <= 1: return n
    return fib(n - 1) + fib(n - 2)

def main():
    iterations = 1000
    start = time.perf_counter()
    
    result = 0
    for _ in range(iterations):
        result += fib(19)
        
    end = time.perf_counter()
    
    total_ms = (end - start) * 1000.0
    avg_us = (total_ms * 1000.0) / iterations
    
    print("--- Python Benchmark ---")
    print(f"Fibonacci(19) runs: {iterations}")
    print(f"Total time: {total_ms:.2f} ms")
    print(f"Time per execution: {total_ms / iterations:.2f} ms")
    print(f"Sanity check result sum: {result}")

if __name__ == "__main__":
    main()
