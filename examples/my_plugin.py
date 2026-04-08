import sys
import json

def process_syscall(req_id, data):
    sys.stderr.write(f"Received JSON for Syscall {req_id}: " + json.dumps(data) + "\n")
    sys.stderr.flush()
    if "Int32" in data:
        val = data["Int32"]
        return {"Int32": val * 2}
    return "Null"

def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break
            
        line = line.strip()
        if not line:
            continue
            
        try:
            req_id, data = json.loads(line)
            res = process_syscall(req_id, data)
            sys.stdout.write(json.dumps(res) + "\n")
            sys.stdout.flush()
        except Exception as e:
            # If there's a problem, return a TypeMismatch fault (code 0x08)
            sys.stdout.write(json.dumps({"Result": {"Err": 8}}) + "\n")
            sys.stdout.flush()

if __name__ == "__main__":
    main()
