import sys

def main():
    print("=== Python Demo: CLI Calculator ===")
    print("Supported operators: +  -  *  /  %")
    print("Type 'q' to quit.\n")

    while True:
        try:
            a_str = input("Enter first number: ").strip()
            if a_str == "q":
                print("Goodbye.")
                break
            a = int(a_str)
            
            op = input("Enter operator (+  -  *  /  %): ").strip()
            b = int(input("Enter second number: ").strip())

            if op == '+': res = a + b
            elif op == '-': res = a - b
            elif op == '*': res = a * b
            elif op == '/':
                if b == 0:
                    print("Error: division by zero.\n")
                    continue
                res = a // b
            elif op == '%':
                if b == 0:
                    print("Error: division by zero.\n")
                    continue
                res = a % b
            else:
                print("Error: unknown operator.\n")
                continue

            print(f"Result: {a} {op} {b} = {res}\n")

        except ValueError:
            print("Error: invalid number — only digits (and leading '-') allowed.\n")

if __name__ == "__main__":
    main()
