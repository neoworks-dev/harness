import { expect, test } from "bun:test";
import { Context, FiberState } from "@neoworks/extension-system";
import { SlotRegistry } from "../ui/kernel/slots/registry.ts";
import { SlotsService } from "../ui/kernel/slots/service.ts";
import type { Chain, Keyed, List, Single } from "../ui/kernel/slots/types.ts";

declare module "../ui/kernel/slots/types.ts" {
  interface SlotMap {
    "t.list": List;
    "t.single": Single;
    "t.keyed": Keyed<"a" | "b", { n: number }>;
    "t.chain": Chain<{ n: number }>;
    "t.inner": List;
    "t.other": List;
  }
}

/** A stand-in for a Svelte component: the registry only passes it around. */
const comp = (name: string) => Object.assign(() => {}, { displayName: name }) as never;
const names = (occupants: { component: unknown }[]) =>
  occupants.map((o) => (o.component as { displayName: string }).displayName);

/** Why a plugin failed to load; the fibers are thenables, which `rejects` does not take. */
const failure = (fiber: PromiseLike<unknown>) =>
  Promise.resolve(fiber).then(
    () => undefined,
    (error: Error) => error.message,
  );

/** A context with the service mounted on a registry of its own, and `root` taken by an owner. */
async function setup(slots: Record<string, "list" | "single" | "keyed" | "chain">) {
  const registry = new SlotRegistry();
  const root = new Context();
  await root.plugin(SlotsService, registry);
  const owner = root.plugin({
    name: "owner",
    inject: ["slots"],
    apply: (ctx: Context) =>
      ctx.slots.add("root", { component: comp("owner"), slots: slots as never }),
  });
  await owner;
  return { registry, root, owner };
}

test("a list draws its entries in order, ties in the order added", async () => {
  const { registry, root } = await setup({ "t.list": "list" });
  const add = (name: string, order?: number) =>
    root.plugin({
      name,
      inject: ["slots"],
      apply: (ctx: Context) => ctx.slots.add("t.list", { component: comp(name), order }),
    });
  await add("c", 20);
  await add("a", 10);
  await add("b", 10);
  await add("z");
  expect(names(registry.resolve("t.list"))).toEqual(["z", "a", "b", "c"]);
});

test("a single draws its last entry, and the one before it returns when that goes", async () => {
  const { registry, root } = await setup({ "t.single": "single" });
  const mount = (name: string) =>
    root.plugin({
      name,
      inject: ["slots"],
      apply: (ctx: Context) => ctx.slots.add("t.single", { component: comp(name) }),
    });
  await mount("first");
  const second = mount("second");
  await second;
  expect(names(registry.resolve("t.single"))).toEqual(["second"]);
  await second.dispose();
  expect(names(registry.resolve("t.single"))).toEqual(["first"]);
});

test("a keyed slot draws the entry of the key, else its fallback", async () => {
  const registry = new SlotRegistry();
  const root = new Context();
  await root.plugin(SlotsService, registry);
  await root.plugin({
    name: "owner",
    inject: ["slots"],
    apply: (ctx: Context) =>
      ctx.slots.add("root", {
        component: comp("owner"),
        slots: { "t.keyed": { kind: "keyed", fallback: comp("fallback") } },
      }),
  });
  await root.plugin({
    name: "a",
    inject: ["slots"],
    apply: (ctx: Context) => ctx.slots.add("t.keyed", { key: "a", component: comp("A") }),
  });
  expect(names(registry.resolve("t.keyed", "a"))).toEqual(["A"]);
  expect(names(registry.resolve("t.keyed", "b"))).toEqual(["fallback"]);
});

test("a keyed entry needs a key and a chain entry a select", async () => {
  const { registry } = await setup({ "t.keyed": "keyed", "t.chain": "chain" });
  expect(() => registry.add("t.keyed", { component: comp("x") })).toThrow("need a key");
  expect(() => registry.add("t.chain", { component: comp("x") })).toThrow("need a select");
});

test("a chain draws the first entry, by order, that selects the props", async () => {
  const { registry, root } = await setup({ "t.chain": "chain" });
  const mount = (name: string, order: number, accepts: (n: number) => boolean) =>
    root.plugin({
      name,
      inject: ["slots"],
      apply: (ctx: Context) =>
        ctx.slots.add("t.chain", { component: comp(name), order, select: (p) => accepts(p.n) }),
    });
  await mount("big", 2, (n) => n > 10);
  await mount("odd", 1, (n) => n % 2 === 1);
  await mount("any", 3, () => true);
  expect(names(registry.resolve("t.chain", undefined, { n: 11 }))).toEqual(["odd"]);
  expect(names(registry.resolve("t.chain", undefined, { n: 12 }))).toEqual(["big"]);
  expect(names(registry.resolve("t.chain", undefined, { n: 2 }))).toEqual(["any"]);
});

test("registering into an undeclared slot fails the plugin while it loads", async () => {
  const registry = new SlotRegistry();
  const root = new Context();
  await root.plugin(SlotsService, registry);
  const fiber = root.plugin({
    name: "early",
    inject: ["slots"],
    apply: (ctx: Context) => ctx.slots.add("t.list", { component: comp("x") }),
  });
  expect(await failure(fiber)).toContain('slot "t.list" is not declared');
  expect(fiber.state).toBe(FiberState.FAILED);
});

test("a slot cannot be declared twice", async () => {
  const { root } = await setup({ "t.list": "list" });
  const twin = root.plugin({
    name: "twin",
    inject: ["slots"],
    apply: (ctx: Context) =>
      ctx.slots.add("t.list", { component: comp("twin"), slots: { "t.inner": "list" } }),
  });
  await twin;
  const clash = root.plugin({
    name: "clash",
    inject: ["slots"],
    apply: (ctx: Context) =>
      ctx.slots.add("t.list", { component: comp("clash"), slots: { "t.inner": "list" } }),
  });
  expect(await failure(clash)).toContain('slot "t.inner" is already declared');
});

test("declaring is claiming: an entry's slots exist while it is registered, and go with it", async () => {
  const { registry, root } = await setup({ "t.list": "list" });
  expect(registry.has("t.inner")).toBe(false);
  const panel = root.plugin({
    name: "panel",
    inject: ["slots"],
    apply: (ctx: Context) =>
      ctx.slots.add("t.list", { component: comp("panel"), slots: { "t.inner": "list" } }),
  });
  await panel;
  expect(registry.has("t.inner")).toBe(true);
  const inner = root.plugin({
    name: "inner",
    inject: ["slots"],
    apply: (ctx: Context) => ctx.slots.add("t.inner", { component: comp("inner") }),
  });
  await inner;
  expect(names(registry.resolve("t.inner"))).toEqual(["inner"]);

  await panel.dispose();
  expect(registry.has("t.inner")).toBe(false);
  expect(registry.resolve("t.inner")).toEqual([]);
  expect(inner.state).toBe(FiberState.ACTIVE);
  await inner.dispose();
});

test("inject registers when the owner is mounted, again when it comes back, and not after unload", async () => {
  const registry = new SlotRegistry();
  const root = new Context();
  await root.plugin(SlotsService, registry);
  let runs = 0;
  const contributor = root.plugin({
    name: "contributor",
    inject: ["slots"],
    apply: (ctx: Context) =>
      ctx.slots.inject("t.list", (slot) => {
        runs += 1;
        slot.add({ component: comp("from-inject") });
      }),
  });
  await contributor;
  // Nobody has declared the slot yet: nothing failed, nothing ran.
  expect(runs).toBe(0);

  const mountOwner = () =>
    root.plugin({
      name: "owner",
      inject: ["slots"],
      apply: (ctx: Context) =>
        ctx.slots.add("root", { component: comp("owner"), slots: { "t.list": "list" } }),
    });
  const owner = mountOwner();
  await owner;
  expect(names(registry.resolve("t.list"))).toEqual(["from-inject"]);

  await owner.dispose();
  expect(registry.has("t.list")).toBe(false);
  await mountOwner();
  expect(runs).toBe(2);
  expect(names(registry.resolve("t.list"))).toEqual(["from-inject"]);

  await contributor.dispose();
  expect(registry.resolve("t.list")).toEqual([]);
  expect(registry.has("t.list")).toBe(true);
});

test("unloading a plugin takes its entries out and tells the subscribers", async () => {
  const { registry, root } = await setup({ "t.list": "list" });
  let heard = 0;
  registry.subscribe(() => (heard += 1));
  const pane = root.plugin({
    name: "pane",
    inject: ["slots"],
    apply: (ctx: Context) => ctx.slots.add("t.list", { component: comp("pane") }),
  });
  await pane;
  expect(names(registry.resolve("t.list"))).toEqual(["pane"]);
  const before = heard;
  await pane.dispose();
  expect(registry.resolve("t.list")).toEqual([]);
  expect(heard).toBeGreaterThan(before);
});

test("a contributor that throws once the slot exists fails its own plugin, not the owner's", async () => {
  const registry = new SlotRegistry();
  const root = new Context();
  await root.plugin(SlotsService, registry);
  const log = console.error;
  console.error = () => {};
  try {
    await root.plugin({
      name: "bad",
      inject: ["slots"],
      apply: (ctx: Context) =>
        ctx.slots.inject("t.list", () => {
          throw new Error("nope");
        }),
    });
    const owner = root.plugin({
      name: "owner",
      inject: ["slots"],
      apply: (ctx: Context) =>
        ctx.slots.add("root", { component: comp("owner"), slots: { "t.list": "list" } }),
    });
    await owner;
    expect(owner.state).toBe(FiberState.ACTIVE);
    expect(registry.has("t.list")).toBe(true);
  } finally {
    console.error = log;
  }
});

test("replacing a single frees the slots the old one declared for the new one", async () => {
  const { registry, root } = await setup({ "t.single": "single" });
  const mount = (name: string) =>
    root.plugin({
      name,
      inject: ["slots"],
      apply: (ctx: Context) =>
        ctx.slots.add("t.single", { component: comp(name), slots: { "t.inner": "list" } }),
    });
  const first = mount("first");
  await first;
  const second = mount("second");
  await second;
  expect(second.state).toBe(FiberState.ACTIVE);
  expect(names(registry.resolve("t.single"))).toEqual(["second"]);
  await second.dispose();
  expect(names(registry.resolve("t.single"))).toEqual(["first"]);
  expect(registry.has("t.inner")).toBe(true);
});
