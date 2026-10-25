// Connecting and reconnecting, against a fake daemon that tests can drop, replace and make refuse.
import { describe, expect, test } from "bun:test";
import type { ViewSnapshot } from "@neoworks-dev/neo-protocol/types";
import { connect, fixedAddress, NeoError, type DaemonAddress } from "../src/index.ts";
import { cleanupAfterEach } from "./support/cleanup.ts";
import { FakeDaemon } from "./support/fake-daemon.ts";
import { rejection, until } from "./support/wait.ts";

const later = cleanupAfterEach();

function fake(): FakeDaemon {
  const daemon = new FakeDaemon();
  later(() => daemon.stop());
  return daemon;
}

describe("connect", () => {
  test("rejects when the daemon refuses the token", async () => {
    const daemon = fake();
    const attempt = connect({
      client: "test",
      address: fixedAddress({ url: daemon.url, token: "wrong" }),
      reconnect: false,
    });
    expect(await rejection(attempt)).toBeInstanceOf(Error);
  });

  test("the state is open once connect resolves", async () => {
    const daemon = fake();
    const client = await connect({ client: "test", address: fixedAddress(daemon) });
    later(() => client.close());
    expect(client.state.value).toBe("open");
  });
});

describe("reconnecting", () => {
  test("asks for the address again each time, so a daemon on a new port is found", async () => {
    const first = fake();
    const second = fake();
    let current: DaemonAddress = first;
    let asked = 0;
    const client = await connect({
      client: "test",
      address: () => {
        asked += 1;
        return Promise.resolve(current);
      },
    });
    later(() => client.close());
    const snapshots: ViewSnapshot[] = [];
    client.subscribeView("s1", { onSnapshot: (s) => snapshots.push(s), onPatch: () => {} });
    await until(() => snapshots.length === 1, "snapshot");
    expect(asked).toBe(1);

    current = second;
    first.stop();
    await until(() => snapshots.length === 2, "snapshot from the new daemon", 8000);
    expect(asked).toBeGreaterThan(1);
    expect(second.topics()).toContain("view.subscribe");
  });

  test("registers the presence again, and reports a refusal through onError and closes", async () => {
    const daemon = fake();
    const errors: Error[] = [];
    const client = await connect({
      client: "test",
      address: fixedAddress(daemon),
      presence: { app: "demo", kind: "app", open: true },
      onError: (error) => errors.push(error),
    });
    later(() => client.close());

    daemon.refusing.add("app.presence");
    daemon.dropConnections();
    await client.state.waitFor(8000, (state) => state === "closed");
    expect(errors).toHaveLength(1);
    expect(errors[0]).toBeInstanceOf(NeoError);
    expect(errors[0]).toMatchObject({ code: "bad_request" });
  });

  test("keeps trying while the daemon is away, and a listener hears every state", async () => {
    const daemon = fake();
    const client = await connect({ client: "test", address: fixedAddress(daemon) });
    later(() => client.close());
    const states: string[] = [];
    client.state.on((state) => states.push(state));

    daemon.dropConnections();
    await client.state.waitFor(2000, (state) => state === "reconnecting");
    await client.state.waitFor(8000, (state) => state === "open");
    expect(states).toEqual(["reconnecting", "open"]);
  });
});
