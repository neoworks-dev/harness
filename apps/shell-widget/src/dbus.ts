// neoshell's WidgetSource1 on the session bus: the shell gets each topic with Get when the name
// appears, follows Changed, and forwards the widget's calls to Call. Every payload is JSON.
import dbus from "dbus-next";

export const WIDGET_SOURCE_INTERFACE = "dev.neoworks.Shell.WidgetSource1";
export const WIDGET_SOURCE_PATH = "/dev/neoworks/Shell/WidgetSource";

export type CallHandler = (args: unknown) => unknown;

export interface WidgetSource {
  /** Sets a topic's value and tells the shell when it differs from the last one. */
  publish(topic: string, data: unknown): void;
  close(): void;
}

class WidgetSourceInterface extends dbus.interface.Interface {
  readonly values = new Map<string, string>();

  constructor(private readonly calls: Record<string, CallHandler>) {
    super(WIDGET_SOURCE_INTERFACE);
  }

  Get(topic: string): string {
    return this.values.get(topic) ?? "null";
  }

  async Call(name: string, args: string): Promise<string> {
    const handler = this.calls[name];
    if (handler === undefined) {
      throw new dbus.DBusError(`${WIDGET_SOURCE_INTERFACE}.UnknownCall`, `no call named ${name}`);
    }
    return JSON.stringify((await handler(parse(args))) ?? null);
  }

  Changed(topic: string, json: string): [string, string] {
    return [topic, json];
  }
}

WidgetSourceInterface.configureMembers({
  methods: {
    Get: { inSignature: "s", outSignature: "s" },
    Call: { inSignature: "ss", outSignature: "s" },
  },
  signals: { Changed: { signature: "ss" } },
});

/** Owns `name` and serves the source there; rejects if another process has the name. */
export async function exportWidgetSource(
  name: string,
  calls: Record<string, CallHandler>,
): Promise<WidgetSource> {
  const bus = dbus.sessionBus();
  const source = new WidgetSourceInterface(calls);
  bus.export(WIDGET_SOURCE_PATH, source);
  const reply = await bus.requestName(name, dbus.NameFlag.DO_NOT_QUEUE);
  if (
    reply !== dbus.RequestNameReply.PRIMARY_OWNER &&
    reply !== dbus.RequestNameReply.ALREADY_OWNER
  ) {
    bus.disconnect();
    throw new Error(`${name} is already owned on the session bus`);
  }
  return {
    publish(topic, data) {
      const json = JSON.stringify(data);
      if (source.values.get(topic) === json) return;
      source.values.set(topic, json);
      source.Changed(topic, json);
    },
    close() {
      bus.disconnect();
    },
  };
}

function parse(json: string): unknown {
  try {
    const parsed: unknown = JSON.parse(json);
    return parsed;
  } catch {
    return {};
  }
}
