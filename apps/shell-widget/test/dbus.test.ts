import { expect, test } from "bun:test";
import dbus from "dbus-next";
import { exportWidgetSource, WIDGET_SOURCE_INTERFACE, WIDGET_SOURCE_PATH } from "../src/dbus.ts";

interface WidgetSourceProxy extends dbus.ClientInterface {
  Get(topic: string): Promise<string>;
  Call(name: string, args: string): Promise<string>;
}

// Against the real session bus, the way neoshell talks to it; skipped where there is none.
const hasSessionBus = (process.env.DBUS_SESSION_BUS_ADDRESS ?? "") !== "";

test.skipIf(!hasSessionBus)(
  "the shell gets topics, follows changes and calls through WidgetSource1",
  async () => {
    const name = `dev.neoworks.Neo.AgentsTest${process.pid}`;
    const source = await exportWidgetSource(name, { open: (args) => ({ echoed: args }) });
    const shell = dbus.sessionBus();
    try {
      source.publish("usage", { costToday: 1 });
      const proxy = await shell.getProxyObject(name, WIDGET_SOURCE_PATH);
      const widgetSource = proxy.getInterface<WidgetSourceProxy>(WIDGET_SOURCE_INTERFACE);
      expect(JSON.parse(await widgetSource.Get("usage"))).toEqual({ costToday: 1 });
      expect(JSON.parse(await widgetSource.Get("unknown"))).toBeNull();
      expect(
        JSON.parse(await widgetSource.Call("open", JSON.stringify({ session: "s1" }))),
      ).toEqual({
        echoed: { session: "s1" },
      });
      const refused = await widgetSource.Call("delete", "{}").then(
        () => "",
        (error: Error) => error.message,
      );
      expect(refused).toContain("no call named delete");

      const changed = new Promise<[string, string]>((resolve) => {
        widgetSource.on("Changed", (topic: string, json: string) => resolve([topic, json]));
      });
      // Signals only reach a match rule that is in place, so give it a moment.
      await Bun.sleep(100);
      source.publish("usage", { costToday: 1 });
      source.publish("usage", { costToday: 2 });
      expect(await changed).toEqual(["usage", JSON.stringify({ costToday: 2 })]);
    } finally {
      shell.disconnect();
      source.close();
    }
  },
);
