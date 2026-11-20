#!/usr/bin/env python3
"""The training plugin's MCP server (stdio): what its surfaces read and press.

  status  the run as it stands: state, epoch, progress, the latest losses, learning rate, eta
  set     change the run while it goes: lr, paused, stop (train.py reads them every few steps)
  start   start a run (one at a time), with epochs and lr

The run lives in TRAINING_DIR (default /tmp/neo-training). No dependencies: MCP's stdio transport
is JSON-RPC, one message per line.
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DIR = os.environ.get("TRAINING_DIR", "/tmp/neo-training")
STATUS = os.path.join(DIR, "status.json")
CONTROL = os.path.join(DIR, "control.json")
# A status this old with a run that says it runs: the run died.
STALE_SECONDS = 10

TOOLS = [
    {
        "name": "status",
        "description": "The training run as it stands: state (idle, running, paused, done, stopped), epoch of epochs, progress 0..1, the latest losses, learning rate, eta.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
    },
    {
        "name": "set",
        "description": "Change the running training: its learning rate, pause or resume it, or stop it. The run reads it within a few steps.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "lr": {"type": "number", "description": "The learning rate from now on."},
                "paused": {"type": "boolean"},
                "stop": {"type": "boolean"},
            },
        },
    },
    {
        "name": "start",
        "description": "Start a training run, unless one is running.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "epochs": {"type": "integer", "minimum": 1, "maximum": 100},
                "lr": {"type": "number"},
            },
        },
    },
]


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def write_json(path, value):
    os.makedirs(DIR, exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(value, f)
    os.replace(tmp, path)


def duration(seconds):
    if seconds is None:
        return ""
    seconds = int(seconds)
    return f"{seconds // 60}m {seconds % 60:02d}s" if seconds >= 60 else f"{seconds}s"


def status():
    run = read_json(STATUS)
    if not run:
        return {"state": "idle", "epoch": 0, "epochs": 0, "step": 0, "steps": 0, "progress": 0,
                "loss": 0, "losses": [], "labels": [], "lr": 0, "eta": ""}
    state = run.get("state", "idle")
    # A paused run writes nothing while it waits; a running one that stopped writing died.
    if state == "running" and time.time() - run.get("updated", 0) > STALE_SECONDS:
        state = "stopped"
    steps = run.get("steps") or 1
    losses = run.get("losses", [])
    first = run.get("step", 0) - len(losses) + 1
    return {
        "state": state,
        "epoch": run.get("epoch", 0),
        "epochs": run.get("epochs", 0),
        "step": run.get("step", 0),
        "steps": run.get("steps", 0),
        "progress": round(run.get("step", 0) / steps, 4),
        "loss": run.get("loss", 0),
        "losses": losses,
        "labels": [str(first + i) for i in range(len(losses))],
        "lr": round(run.get("lr", 0), 5),
        "eta": duration(run.get("eta_seconds")),
        "w": round(run.get("w", 0), 4),
        "b": round(run.get("b", 0), 4),
    }


def set_(args):
    control = read_json(CONTROL)
    for key in ("lr", "paused", "stop"):
        if key in args and args[key] is not None:
            control[key] = args[key]
    write_json(CONTROL, control)
    return {"control": control}


def start(args):
    current = status()
    if current["state"] in ("running", "paused"):
        return {"started": False, "why": "a run is going on", "state": current["state"]}
    command = [sys.executable, os.path.join(HERE, "train.py"), "--dir", DIR,
               "--epochs", str(int(args.get("epochs") or 8)), "--lr", str(float(args.get("lr") or 0.05))]
    # The run outlives the server: its own session, its output nowhere.
    subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)
    for _ in range(20):
        time.sleep(0.05)
        if status()["state"] == "running":
            break
    return {"started": True}


def call(name, args):
    if name == "status":
        return status()
    if name == "set":
        return set_(args)
    if name == "start":
        return start(args)
    raise ValueError(f"no tool {name}")


def answer(message):
    method = message.get("method")
    params = message.get("params") or {}
    if method == "initialize":
        return {
            "protocolVersion": params.get("protocolVersion", "2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "training", "version": "0.1.0"},
        }
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        try:
            result = call(params.get("name"), params.get("arguments") or {})
        except Exception as error:  # A tool's failure is its result, not the server's.
            return {"content": [{"type": "text", "text": str(error)}], "isError": True}
        return {"content": [{"type": "text", "text": json.dumps(result)}], "structuredContent": result}
    raise LookupError(method)


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if "id" not in message:
            continue  # A notification: nothing to answer.
        reply = {"jsonrpc": "2.0", "id": message["id"]}
        try:
            reply["result"] = answer(message)
        except LookupError as method:
            reply["error"] = {"code": -32601, "message": f"Method not found: {method}"}
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
