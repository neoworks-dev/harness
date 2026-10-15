import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SessionEvent, SessionEventRecord } from "@suite/protocol/types";
import { startTestDaemon, type TestDaemon } from "@suite/daemon/testing";
import { SuiteClient } from "../src/client.ts";

export async function until(check: () => boolean, what: string, timeoutMs = 5000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(10);
  }
}

/** An env pointing at a directory with no daemon in it. */
export function emptyEnv(): Record<string, string> {
  return { SUITE_SOCKET: join(mkdtempSync(join(tmpdir(), "suite-none-")), "bus.sock") };
}

export interface Rig {
  daemon: TestDaemon;
  client: SuiteClient;
}

/** What a subscriber saw, in order. */
export class Recorder {
  readonly records: SessionEventRecord[] = [];
  live = 0;

  onEvent = (_event: SessionEvent, record: SessionEventRecord): void => {
    this.records.push(record);
  };

  onLive = (): void => {
    this.live += 1;
  };

  get seqs(): number[] {
    return this.records.map((r) => r.seq);
  }

  types(): string[] {
    return this.records.map((r) => r.event.type);
  }

  text(): string {
    let text = "";
    for (const { event } of this.records)
      if (event.type === "agent_message_chunk") text += event.text;
    return text;
  }

  turnEnds(): string[] {
    const reasons: string[] = [];
    for (const { event } of this.records)
      if (event.type === "turn_ended") reasons.push(event.stopReason);
    return reasons;
  }

  turnsEnded(): number {
    return this.types().filter((t) => t === "turn_ended").length;
  }
}

export async function start(): Promise<Rig> {
  const daemon = await startTestDaemon();
  const client = await SuiteClient.connect({ client: "test", env: daemon.env });
  return { daemon, client };
}
