import { expect, test } from "bun:test";
import { initialStatus, type SessionStatus } from "../ui/lib/status.ts";
import {
  approvalsTone,
  contextPercent,
  formatCost,
  formatElapsed,
  statusSegments,
} from "../ui/plugins/statusline/segments.ts";

const idle = { branch: "main", queued: 0, connection: "open", now: 0 } as const;

test("elapsed time in seconds, minutes and hours", () => {
  expect(formatElapsed(7_400)).toBe("7s");
  expect(formatElapsed(125_000)).toBe("2m 05s");
  expect(formatElapsed(3_780_000)).toBe("1h 03m");
  expect(formatElapsed(-5)).toBe("0s");
});

test("approvals are coloured by how much they let through", () => {
  expect(approvalsTone("ask")).toBe("muted");
  expect(approvalsTone("review")).toBe("accent");
  expect(approvalsTone("edits")).toBe("accent");
  expect(approvalsTone("auto")).toBe("danger");
});

test("context use is a percentage, when both numbers are known", () => {
  expect(contextPercent({ type: "usage", contextUsed: 25_000, contextSize: 200_000 })).toBe(13);
  expect(contextPercent({ type: "usage", contextUsed: 5 })).toBeUndefined();
  expect(contextPercent(undefined)).toBeUndefined();
  expect(contextPercent({ type: "usage", contextUsed: 5, contextSize: 0 })).toBeUndefined();
});

test("cost in dollars", () => {
  expect(formatCost(1.2)).toBe("$1.20");
  expect(formatCost(0.0004)).toBe("<$0.01");
  expect(formatCost(0)).toBe("$0.00");
  expect(formatCost(undefined)).toBeUndefined();
});

test("an idle session shows branch, harness, the harness's settings and approvals", () => {
  const status: SessionStatus = {
    ...initialStatus(),
    harness: "claude",
    approvals: "auto",
    settings: { claude: { model: "sonnet", effort: "high" }, pi: { model: "other" } },
  };
  const segments = statusSegments({ ...idle, status });
  expect(segments.map((s) => s.text)).toEqual(["main", "claude", "sonnet", "high", "auto"]);
  expect(segments.at(-1)?.tone).toBe("danger");
});

test("a running turn shows elapsed time, the queue, usage and a lost connection", () => {
  const status: SessionStatus = {
    ...initialStatus(),
    running: true,
    turnStartedAt: 1_000,
    usage: { type: "usage", contextUsed: 190_000, contextSize: 200_000, cost: 0.5 },
  };
  const segments = statusSegments({
    ...idle,
    branch: null,
    status,
    queued: 2,
    connection: "reconnecting",
    now: 66_000,
  });
  expect(segments.map((s) => s.text)).toEqual([
    "pi",
    "ask",
    "working 1m 05s",
    "2 queued",
    "95% context",
    "$0.50",
    "reconnecting",
  ]);
  expect(segments.find((s) => s.text === "95% context")?.tone).toBe("danger");
});
