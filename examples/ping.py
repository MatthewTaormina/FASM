import sys

def main():
    # Pure generic ping-pong server.
    # Reads a line from STDIN, immediately writes it to STDOUT.
    # This measures absolute theoretical IPC limit.
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        sys.stdout.write(line)
        sys.stdout.flush()

if __name__ == "__main__":
    main()
