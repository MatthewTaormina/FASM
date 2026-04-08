import sys
import json

def fib_tco(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a

def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        try:
            # FASM sends: [req_id, { "Int32": value }] if passed directly from an INT32 slot
            # or just the value depending on how it's encoded. 
            # Actually, the sidecar bridge serializes the Value enum.
            req_id, data = json.loads(line)
            
            # Extract n from the FASM Value JSON
            n = data.get("Int32", 0)
            
            result = fib_tco(n)
            
            # Return as Int32 value
            sys.stdout.write(json.dumps({"Int32": result}) + "\n")
            sys.stdout.flush()
        except:
            break

if __name__ == "__main__":
    main()
