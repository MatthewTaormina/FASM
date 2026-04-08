#include <stdio.h>

long long fib(long long n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

int main() {
    // Prevent optimization out by printing it
    printf("%lld\n", fib(19));
    return 0;
}
