const readline = require('readline');

const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout
});

console.log("=== Node.js Demo: CLI Calculator ===");
console.log("Supported operators: +  -  *  /  %");
console.log("Type 'q' to quit.\n");

function ask() {
    rl.question("Enter first number: ", (aStr) => {
        aStr = aStr.trim();
        if (aStr === 'q') {
            console.log("Goodbye.");
            rl.close();
            return;
        }

        const a = parseInt(aStr, 10);
        if (isNaN(a)) {
            console.log("Error: invalid number — only digits (and leading '-') allowed.\n");
            return ask();
        }

        rl.question("Enter operator (+  -  *  /  %): ", (op) => {
            op = op.trim();
            rl.question("Enter second number: ", (bStr) => {
                const b = parseInt(bStr.trim(), 10);
                if (isNaN(b)) {
                    console.log("Error: invalid number — only digits (and leading '-') allowed.\n");
                    return ask();
                }

                let res;
                if (op === '+') res = a + b;
                else if (op === '-') res = a - b;
                else if (op === '*') res = a * b;
                else if (op === '/') {
                    if (b === 0) { console.log("Error: division by zero.\n"); return ask(); }
                    res = Math.floor(a / b);
                }
                else if (op === '%') {
                    if (b === 0) { console.log("Error: division by zero.\n"); return ask(); }
                    res = a % b;
                } else {
                    console.log("Error: unknown operator.\n");
                    return ask();
                }

                console.log(`Result: ${a} ${op} ${b} = ${res}\n`);
                ask();
            });
        });
    });
}

ask();
