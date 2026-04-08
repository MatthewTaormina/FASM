function fib(n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

function main() {
    const iterations = 1000;
    
    // Warmup the JIT compiler slightly to get realistic 'warm' metrics
    for(let i=0; i<10; i++) fib(19);

    const start = process.hrtime.bigint();
    
    let result = 0;
    for (let i = 0; i < iterations; i++) {
        result += fib(19);
    }
    
    const end = process.hrtime.bigint();
    const totalMs = Number(end - start) / 1000000.0;
    const avgMs = totalMs / iterations;
    
    console.log("--- Node.js Benchmark ---");
    console.log(`Fibonacci(19) runs: ${iterations}`);
    console.log(`Total time: ${totalMs.toFixed(2)} ms`);
    console.log(`Time per execution: ${avgMs.toFixed(2)} ms`);
    console.log(`Sanity check result sum: ${result}`);
}

main();
