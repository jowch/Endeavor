#!/usr/bin/env python3
"""julia-smoke.py ENDEAVOR [--helper]: run one real Julia notebook through the
app's runtime, as an agent would. Starts `ENDEAVOR [--helper] mcp` (the
stdio MCP server the plugin runs; the app's exe is the helper with
`--helper`), makes a notebook, runs a cell, reads its output, closes the
notebook and stops the runtime. Fails, with what the runtime said, if a step
doesn't do what it should. Used by .github/workflows/windows-julia.yml, and
runs on any platform."""

import json
import os
import subprocess
import sys
import tempfile
import threading
import time

# A first run starts Julia and installs Pluto into the depot: minutes.
STEP_TIMEOUT = 1200


def main():
    exe, extra = os.path.abspath(sys.argv[1]), sys.argv[2:]
    folder = tempfile.mkdtemp(prefix="endeavor-smoke-")
    path = os.path.join(folder, "smoke.jl")
    server = subprocess.Popen([exe, *extra, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, cwd=folder)
    replies = {}
    ready = threading.Condition()

    def read():
        for line in server.stdout:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if "id" in message and ("result" in message or "error" in message):
                with ready:
                    replies[message["id"]] = message
                    ready.notify_all()

    threading.Thread(target=read, daemon=True).start()
    ids = iter(range(1, 1000))

    def send(message):
        server.stdin.write((json.dumps(message) + "\n").encode())
        server.stdin.flush()

    def request(method, params):
        id = next(ids)
        send({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        deadline = time.monotonic() + STEP_TIMEOUT
        with ready:
            while id not in replies:
                if server.poll() is not None:
                    fail(f"{method}: the server exited ({server.returncode})")
                if not ready.wait(timeout=5) and time.monotonic() > deadline:
                    fail(f"{method}: no answer in {STEP_TIMEOUT} s")
        reply = replies.pop(id)
        if "error" in reply:
            fail(f"{method}: {reply['error']}")
        return reply["result"]

    def call(tool, **arguments):
        started = time.monotonic()
        while True:
            result = request("tools/call", {"name": tool, "arguments": arguments})
            text = "".join(c.get("text", "") for c in result.get("content", []))
            try:
                answer = json.loads(text)
            except ValueError:
                answer = text
            # Julia is starting: the runtime says to make the same call again.
            starting = isinstance(answer, dict) and str(answer.get("error", "")).endswith("_starting")
            if not starting or time.monotonic() - started > STEP_TIMEOUT:
                break
            print(f"{tool}: {answer['error']} ({time.monotonic() - started:.0f} s)", flush=True)
        print(f"{tool} ({time.monotonic() - started:.0f} s): {text[:400]}", flush=True)
        if result.get("isError") or (isinstance(answer, dict) and "error" in answer):
            fail(f"{tool} failed: {text}")
        return answer

    try:
        request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "julia-smoke", "version": "1"}})
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        made = call("new_notebook", path=path)
        notebook = find(made, "notebook_id") or fail(f"new_notebook gave no notebook_id: {made}")
        # A new notebook has one empty cell; an edit needs a read first.
        order = call("get_cell_order", notebook_id=notebook)
        cells = find(order, "cell_ids") or find(order, "cells") or fail(f"get_cell_order gave no cells: {order}")
        cell = cells[0] if isinstance(cells[0], str) else find(cells[0], "cell_id")
        call("read_cell", notebook_id=notebook, cell_id=cell)
        call("edit_cell", notebook_id=notebook, cell_id=cell, code="smoke = sum(1:10) * 2", run_after=True)
        # The first run precompiles first; the cell waits in the queue meanwhile.
        started = time.monotonic()
        while True:
            read_back = call("read_cell", notebook_id=notebook, cell_id=cell)
            if not (read_back.get("running") or read_back.get("queued")) or time.monotonic() - started > STEP_TIMEOUT:
                break
            time.sleep(5)
        if read_back.get("errored") or read_back.get("output") != "110":
            fail(f"the cell's output isn't 110: {read_back}")
        call("close_notebook", notebook_id=notebook)
        print("A Julia notebook ran through the runtime.", flush=True)
    finally:
        server.stdin.close()
        try:
            server.wait(timeout=30)
        except subprocess.TimeoutExpired:
            server.kill()
        subprocess.run([exe, *extra, "stop"], timeout=120)


def find(value, key):
    """The first `key` anywhere in a tool's JSON answer."""
    if isinstance(value, dict):
        if key in value:
            return value[key]
        value = list(value.values())
    if isinstance(value, list):
        for item in value:
            found = find(item, key)
            if found:
                return found
    return None


def fail(why):
    print(f"::error::{why}", flush=True)
    sys.exit(1)


if __name__ == "__main__":
    main()
