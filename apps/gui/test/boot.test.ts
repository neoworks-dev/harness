import { expect, test } from "bun:test";
import { Context } from "@neoworks/extension-system";
import { boot } from "../ui/kernel/boot.ts";

/** Why the boot failed; `rejects` is typed as returning nothing to await. */
const reason = (booted: Promise<void>): Promise<string> =>
  booted.then(
    () => "",
    (error: Error) => error.message,
  );

test("services mount before plugins, each in the order listed", async () => {
  const order: string[] = [];
  const named = (name: string): { name: string; apply: () => void } => ({
    name,
    apply: () => void order.push(name),
  });
  await boot(new Context(), [named("s1"), named("s2")], [named("p1"), named("p2")]);
  expect(order).toEqual(["s1", "s2", "p1", "p2"]);
});

test("a plugin that fails is left out, the rest still mount, and the boot says which failed", async () => {
  const mounted: string[] = [];
  const booted = boot(
    new Context(),
    [],
    [
      {
        name: "broken",
        apply() {
          throw new Error("boom");
        },
      },
      { name: "fine", apply: () => void mounted.push("fine") },
    ],
  );
  expect(await reason(booted)).toContain("plugin broken failed: Error: boom");
  expect(mounted).toEqual(["fine"]);
});

test("a plugin may need a service a later one provides", async () => {
  const seen: string[] = [];
  await boot(
    new Context(),
    [],
    [
      { name: "user", inject: ["thing"], apply: () => void seen.push("user") },
      { name: "maker", apply: (ctx: Context) => void ctx.provide("thing", {}) },
    ],
  );
  expect(seen).toEqual(["user"]);
});

test("a plugin whose service never comes is reported", async () => {
  const booted = boot(new Context(), [], [{ name: "lonely", inject: ["nothing"], apply() {} }]);
  expect(await reason(booted)).toContain("plugin lonely is waiting for a service");
});
