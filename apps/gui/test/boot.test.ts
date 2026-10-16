import { expect, test } from "bun:test";
import { Context } from "@neoworks/extension-system";
import { boot } from "../ui/kernel/boot.ts";

test("services mount before plugins, each in the order listed", async () => {
  const order: string[] = [];
  const named = (name: string) => ({ name, apply: () => void order.push(name) });
  await boot(new Context(), [named("s1"), named("s2")], [named("p1"), named("p2")]);
  expect(order).toEqual(["s1", "s2", "p1", "p2"]);
});

test("a plugin that fails is left out and the rest still mount", async () => {
  const root = new Context();
  const mounted: string[] = [];
  const errors: unknown[][] = [];
  const log = console.error;
  console.error = (...args: unknown[]) => void errors.push(args);
  try {
    await boot(
      root,
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
  } finally {
    console.error = log;
  }
  expect(mounted).toEqual(["fine"]);
  expect(errors).toHaveLength(1);
  expect(String(errors[0]?.[0])).toContain("broken");
});

test("a plugin may need a service a later one provides", async () => {
  const root = new Context();
  const seen: string[] = [];
  const log = console.error;
  console.error = () => seen.push("warned");
  try {
    await boot(
      root,
      [],
      [
        { name: "user", inject: ["thing"], apply: () => void seen.push("user") },
        { name: "maker", apply: (ctx: Context) => void ctx.provide("thing", {}) },
      ],
    );
  } finally {
    console.error = log;
  }
  expect(seen).toEqual(["user"]);
});
