#!/usr/bin/env python3
"""
db_query.py — Demo database sidecar plugin for fasm-engine.

Handles syscall IDs 100, 101, 102 from FASM executions.
Uses the same JSON-over-stdin/stdout IPC protocol as other FASM sidecars.

Protocol:
  stdin:  JSON line  [req_id, value]
  stdout: JSON line  value
"""

import sys
import json

# Simulated in-memory database
USERS = {
    1: {"id": 1, "name": "Alice", "email": "alice@example.com"},
    2: {"id": 2, "name": "Bob",   "email": "bob@example.com"},
}

ORDERS = []
next_order_id = 1


def handle(syscall_id, data):
    global next_order_id

    if syscall_id == 100:
        # DB_QUERY: SELECT user by id
        user_id = data.get("Int32", 0) if isinstance(data, dict) else 0
        user = USERS.get(user_id)
        if user:
            return {"Struct": {str(k): {"Str": v} if isinstance(v, str) else {"Int32": v}
                               for k, v in enumerate(user.values())}}
        return {"Null": None}

    elif syscall_id == 101:
        # DB_INSERT: insert order
        order = {"id": next_order_id, "data": str(data)}
        ORDERS.append(order)
        next_order_id += 1
        return {"Int32": order["id"]}

    elif syscall_id == 102:
        # DB_LIST_ORDERS
        return {"Int32": len(ORDERS)}

    return {"Null": None}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req_id, syscall_id, value = json.loads(line)
            result = handle(syscall_id, value)
            sys.stdout.write(json.dumps(result) + "\n")
            sys.stdout.flush()
        except Exception as e:
            sys.stderr.write(f"db_query error: {e}\n")
            sys.stdout.write(json.dumps({"Null": None}) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
