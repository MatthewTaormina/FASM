#include <stdio.h>
#include <windows.h>

long long fib(long long n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

int main(int argc, char* argv[]) {
    int iterations = 1000;
    long long target = 19;
    if (argc > 1) {
        target = atoi(argv[1]);
    }
    long long result = 0;
    
    LARGE_INTEGER frequency;
    LARGE_INTEGER start, end;
    
    QueryPerformanceFrequency(&frequency);
    QueryPerformanceCounter(&start);
    
    for (int i = 0; i < iterations; i++) {
        result += fib(target);
    }
    
    QueryPerformanceCounter(&end);
    
    double totalTimeMs = (double)(end.QuadPart - start.QuadPart) * 1000.0 / frequency.QuadPart;
    double avgTimeUs = (totalTimeMs * 1000.0) / iterations;
    
    printf("--- Native C Benchmark ---\n");
    printf("Fibonacci(19) runs: %d\n", iterations);
    printf("Total time: %.2f ms\n", totalTimeMs);
    printf("Time per execution: %.2f us (microseconds)\n", avgTimeUs);
    printf("Sanity check result sum: %lld\n", result);
    
    return 0;
}
