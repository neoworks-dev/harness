#!/usr/bin/env python3
"""A small training run that can be watched and steered while it runs.

It fits y = 3x + 2 (with noise) by minibatch gradient descent, slowly enough to watch. Every step
it writes its status to <dir>/status.json; every few steps it reads <dir>/control.json, where the
learning rate can be changed and the run paused or stopped. The training plugin's server
(server.py) reads the one and writes the other; the surfaces draw them. No dependencies.

    python3 train.py --dir /tmp/neo-training --epochs 8 --lr 0.05
"""
import argparse
import json
import os
import random
import time

CONTROL_EVERY = 5
HISTORY = 120


def write_json(path, value):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(value, f)
    os.replace(tmp, path)


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dir", default=os.environ.get("TRAINING_DIR", "/tmp/neo-training"))
    parser.add_argument("--epochs", type=int, default=8)
    parser.add_argument("--steps", type=int, default=40, help="steps per epoch")
    parser.add_argument("--lr", type=float, default=0.05)
    parser.add_argument("--delay", type=float, default=0.15, help="seconds per step")
    args = parser.parse_args()
    os.makedirs(args.dir, exist_ok=True)
    status_path = os.path.join(args.dir, "status.json")
    control_path = os.path.join(args.dir, "control.json")
    write_json(control_path, {"lr": args.lr, "paused": False, "stop": False})

    rng = random.Random(7)
    data = [(x, 3 * x + 2 + rng.gauss(0, 0.3)) for x in (rng.uniform(-1, 1) for _ in range(512))]
    w, b, lr = rng.uniform(-1, 1), rng.uniform(-1, 1), args.lr
    total = args.epochs * args.steps
    losses = []
    started = time.time()
    paused_for = 0.0
    state = "running"

    def report(step, epoch, loss):
        elapsed = time.time() - started - paused_for
        per_step = elapsed / step if step else 0
        write_json(status_path, {
            "state": state,
            "epoch": epoch,
            "epochs": args.epochs,
            "step": step,
            "steps": total,
            "loss": loss,
            "losses": losses[-HISTORY:],
            "lr": lr,
            "w": w,
            "b": b,
            "eta_seconds": round(per_step * (total - step)) if state == "running" else None,
            "pid": os.getpid(),
            "updated": time.time(),
        })

    step = 0
    loss = 0.0
    for epoch in range(1, args.epochs + 1):
        for _ in range(args.steps):
            if step % CONTROL_EVERY == 0:
                control = read_json(control_path)
                lr = float(control.get("lr", lr))
                if control.get("stop"):
                    state = "stopped"
                    report(step, epoch, loss)
                    return
                if control.get("paused"):
                    state = "paused"
                    report(step, epoch, loss)
                    pause_start = time.time()
                    while read_json(control_path).get("paused") and not read_json(control_path).get("stop"):
                        time.sleep(0.25)
                    paused_for += time.time() - pause_start
                    state = "running"
                    control = read_json(control_path)
                    lr = float(control.get("lr", lr))
                    if control.get("stop"):
                        state = "stopped"
                        report(step, epoch, loss)
                        return
            batch = rng.sample(data, 32)
            grad_w = grad_b = loss = 0.0
            for x, y in batch:
                err = w * x + b - y
                loss += err * err / len(batch)
                grad_w += 2 * err * x / len(batch)
                grad_b += 2 * err / len(batch)
            w -= lr * grad_w
            b -= lr * grad_b
            step += 1
            losses.append(round(loss, 5))
            report(step, epoch, round(loss, 5))
            time.sleep(args.delay)
    state = "done"
    report(step, args.epochs, round(loss, 5))


if __name__ == "__main__":
    main()
