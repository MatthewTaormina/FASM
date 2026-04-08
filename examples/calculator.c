/**
 * calculator.c — CLI Calculator (C Reference Implementation)
 *
 * Companion to examples/calculator.fasm.
 * Demonstrates: I/O, control flow, scoped functions, error handling.
 *
 * Build (GCC/Clang):
 *   gcc -o calculator calculator.c && ./calculator
 *
 * The FASM equivalent (calculator.fasm) implements the same logic
 * instruction-by-instruction using only FASM primitives.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── Error codes (mirrors FASM RESULT fault codes) ─────────────────────────── */
#define ERR_BAD_INPUT      1
#define ERR_DIV_BY_ZERO    2
#define ERR_UNKNOWN_OP     3

/* ── parse_int ──────────────────────────────────────────────────────────────
 * Converts an ASCII string to an integer.
 * Returns 0 on success, error code on failure.
 * Mirrors the manual byte-by-byte ASCII parsing in calculator.fasm.
 */
int parse_int(const char *s, int *out) {
    int result = 0;
    int sign   = 1;
    int i      = 0;

    /* Optional leading '-' */
    if (s[0] == '-') {
        sign = -1;
        i    = 1;
    }

    /* Must have at least one digit */
    if (s[i] == '\0') return ERR_BAD_INPUT;

    for (; s[i] != '\0'; i++) {
        char c = s[i];

        /* Validate digit (ASCII 48–57) */
        if (c < '0' || c > '9') return ERR_BAD_INPUT;

        result = result * 10 + (c - '0');
    }

    *out = result * sign;
    return 0;
}

/* ── calculate ──────────────────────────────────────────────────────────────
 * Performs one arithmetic operation.
 * Returns 0 on success, error code on failure.
 * Mirrors FUNC Calculate in calculator.fasm.
 */
int calculate(int a, int b, char op, int *result) {
    switch (op) {
        case '+': *result = a + b;  return 0;
        case '-': *result = a - b;  return 0;
        case '*': *result = a * b;  return 0;
        case '/':
            if (b == 0) return ERR_DIV_BY_ZERO;
            *result = a / b;
            return 0;
        case '%':
            if (b == 0) return ERR_DIV_BY_ZERO;
            *result = a % b;
            return 0;
        default:
            return ERR_UNKNOWN_OP;
    }
}

/* ── print_error ────────────────────────────────────────────────────────────
 * Prints a human-readable error message.
 * Mirrors FUNC PrintError in calculator.fasm.
 */
void print_error(int code) {
    switch (code) {
        case ERR_BAD_INPUT:   printf("Error: invalid number — only digits (and leading '-') allowed.\n"); break;
        case ERR_DIV_BY_ZERO: printf("Error: division by zero.\n");                                       break;
        case ERR_UNKNOWN_OP:  printf("Error: unknown operator. Use +  -  *  /  %%\n");                    break;
        default:              printf("Error: unknown fault (code %d).\n", code);                          break;
    }
}

/* ── main ───────────────────────────────────────────────────────────────────
 * Entry point — interactive REPL loop.
 * Mirrors FUNC Main in calculator.fasm.
 */
int main(void) {
    char buf_a[64], buf_b[64], buf_op[8];
    int  a, b, result, err;

    printf("=== FASM Demo: CLI Calculator (C Reference) ===\n");
    printf("Supported operators: +  -  *  /  %%\n");
    printf("Type 'q' to quit.\n\n");

    /* REPL loop — mirrors the JMP MainLoop / LABEL MainLoop structure in FASM */
    while (1) {
        printf("Enter expression (a op b): ");
        fflush(stdout);

        /* Read first operand */
        if (scanf("%63s", buf_a) != 1) break;
        if (buf_a[0] == 'q') break;

        /* Read operator */
        if (scanf("%7s", buf_op) != 1) break;

        /* Read second operand */
        if (scanf("%63s", buf_b) != 1) break;

        /* Parse numbers — error handling via RESULT-style error codes */
        err = parse_int(buf_a, &a);
        if (err != 0) { print_error(err); continue; }

        err = parse_int(buf_b, &b);
        if (err != 0) { print_error(err); continue; }

        /* Single char operator */
        char op = buf_op[0];

        /* Perform calculation — scoped function with error return */
        err = calculate(a, b, op, &result);
        if (err != 0) {
            print_error(err);
            continue;
        }

        printf("Result: %d %c %d = %d\n\n", a, op, b, result);
    }

    printf("Goodbye.\n");
    return 0;
}
