/**
 * fibbench.c — Fibonacci Benchmark (C Reference)
 *
 * Mirrors FASM examples/fibonacci.fasm exactly:
 *   - Fibonacci(30) — same N as the FASM benchmark
 *   - Same tail-call-equivalent iterative accumulator algorithm:
 *       fib(n, a=0, b=1) → iterate n times, return a
 *   - 50,000 iterations — same as `fasm bench fibonacci.fasmc 50000`
 *
 * Build (MSVC):
 *   cl /O2 /Fe:fibbench.exe fibbench.c
 * Build (GCC/Clang):
 *   gcc -O2 -o fibbench fibbench.c
 *
 * Expected result: fib(30) = 832040
 */

#include <stdio.h>
#include <windows.h>

/* Mirrors FASM FUNC Fibonacci with TAIL_CALL:
 *   Fibonacci(n, a, b):
 *     if n == 0: return a
 *     if n == 1: return b
 *     Fibonacci(n-1, b, a+b)
 *
 * Equivalent iterative form:
 */
static long long fib_tco(int n) {
    long long a = 0, b = 1;
    while (n > 1) {
        long long next_b = a + b;
        a = b;
        b = next_b;
        n--;
    }
    if (n == 0) return a;
    return b;
}

int main(void) {
    const int    TARGET     = 30;
    const int    ITERATIONS = 50000;
    long long    result     = 0;

    LARGE_INTEGER frequency, start, end;
    QueryPerformanceFrequency(&frequency);
    QueryPerformanceCounter(&start);

    for (int i = 0; i < ITERATIONS; i++) {
        result += fib_tco(TARGET);
    }

    QueryPerformanceCounter(&end);

    double total_ms = (double)(end.QuadPart - start.QuadPart) * 1000.0 / frequency.QuadPart;
    double avg_us   = (total_ms * 1000.0) / ITERATIONS;

    printf("--- Native C Benchmark (matches FASM fibonacci.fasm) ---\n");
    printf("Algorithm : iterative accumulator (mirrors FASM TAIL_CALL)\n");
    printf("N         : %d\n", TARGET);
    printf("Iterations: %d\n", ITERATIONS);
    printf("Total time: %.2f ms\n", total_ms);
    printf("Time/iter : %.2f us (microseconds)\n", avg_us);
    printf("Result    : %lld (sanity: should be 832040 x %d = %lld)\n",
           result / ITERATIONS, ITERATIONS, (long long)832040 * ITERATIONS);

    return 0;
}
