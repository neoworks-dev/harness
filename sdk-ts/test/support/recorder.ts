import type { SessionEvent, SessionEventRecord } from "@suite/protocol/types";

/** Pass it to `subscribe` as the options; it keeps what the subscriber saw, in order. */
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
    return this.records.map((record) => record.seq);
  }

  types(): string[] {
    return this.records.map((record) => record.event.type);
  }

  /** The agent's reply, chunks joined. */
  text(): string {
    let text = "";
    for (const { event } of this.records) {
      if (event.type === "agent_message_chunk") text += event.text;
    }
    return text;
  }

  turnEnds(): string[] {
    const reasons: string[] = [];
    for (const { event } of this.records) {
      if (event.type === "turn_ended") reasons.push(event.stopReason);
    }
    return reasons;
  }

  turnsEnded(): number {
    return this.turnEnds().length;
  }
}
