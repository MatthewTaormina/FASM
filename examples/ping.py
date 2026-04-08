import sys
import json

def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        # FASM sends: [req_id, value]
        req_id, data = json.loads(line)
        # FASM expects back: value
        sys.stdout.write(json.dumps(data) + "\n")
        sys.stdout.flush()

if __name__ == "__main__":
    main()
