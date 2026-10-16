# The window's plugin kernel

Every pane of the window is a plugin on [`@neoworks/extension-system`](../../../../../extension-system/README.md) (read its `skills/extension-system/SKILL.md` first: `ctx.effect`, `inject`, the fiber lifecycle). Plugins are compiled into the bundle; nothing is loaded at run time, and external apps still only get templates.

```
kernel/          the root Context, boot, host services, slots, keymap, commands
plugins/         one folder per pane; index.ts lists them in mount order
plugins/legacy/  panes not converted yet, mounted into the shell's slots as they were drawn
```

`main.ts` calls `boot(root, services, plugins)`: the host services mount first (`kernel/services.ts`), then the plugins in the order of `plugins/index.ts`, then the window opens. `App.svelte` only renders the `root` slot; the `shell` plugin fills it.

## A pane plugin

A folder in `plugins/<name>/` with its components, its state and an `index.ts`:

```ts
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts"; // the slot names it registers into
import Sessions from "./Sessions.svelte";

export const sessionsSidebar = {
  name: "sessions",
  inject: ["slots", "daemon", "keymap", "commands"], // reading anything else on ctx throws
  apply(ctx: Context) {
    const { slots, keymap, commands } = ctx;
    slots.inject("shell.left", (slot) => {
      slot.add({ component: Sessions });
    });
    keymap.bind("ctrl-s", () => void toggle());
    commands.register({ name: "sidebar", desc: "…", run: toggle });
  },
};
```

Add it to `plugins/index.ts`. Everything registered goes through `ctx.effect` inside the services, so unloading the plugin undoes it. For a raw API (`setInterval`, a listener) wrap it yourself: `ctx.effect(() => { …; return () => undo(); })`.

`plugins/sessions` (sidebar, keys, state) and `plugins/statusline` (declares a slot another plugin fills) are the worked examples.

## Slots

A slot is declared on `SlotMap` by augmenting `kernel/slots/types.ts`, so names, kinds and props are type-checked in `<Slot>` and when registering:

```ts
declare module "../../kernel/slots/types.ts" {
  interface SlotMap {
    "shell.left": List;
    "chat.item": Keyed<"text" | "tool", { item: ViewItem }>;
  }
}
```

Props are type literals (`{ item: ViewItem }`), not interfaces. Four kinds:

| Kind     | Draws                                                        | Entry takes                     |
| -------- | ------------------------------------------------------------ | ------------------------------- |
| `Single` | the last entry added; removing it brings back the previous   | `component`                     |
| `List`   | every entry, by `order` (ties: order added)                  | `component`, `order?`           |
| `Keyed`  | the entry whose `key` equals the `<Slot key>`                | `component`, `key`              |
| `Chain`  | the first entry, by `order`, whose `select(props)` is truthy | `component`, `order?`, `select` |

A `Keyed` or `Chain` slot draws its `fallback` (set where the slot is declared) when no entry claims it; `<Slot name="shell.main" />`, `<Slot name="chat.item" key={kind} props={{ item }} />`. `select` may read reactive state: the slot redraws when it changes.

**Declaring is claiming.** An entry declares the child slots its component renders, with their kinds, and they exist while the entry is registered (for a `Single`, while it is the one drawn):

```ts
slots.add("root", {
  component: Shell,
  slots: { "shell.left": "list", "shell.main": { kind: "chain", fallback: Empty } },
});
```

Only a declared slot takes entries. `slots.add("x", …)` into one that is not declared throws, and the plugin fails to load. When the declaring entry unloads, its slots and everything registered into them go too. A slot cannot be declared twice.

**Register with `slots.inject`**, not `slots.add`, unless you are the plugin that mounts the slot's parent. `inject(name, (slot) => { slot.add(…) })` runs the callback now if the slot is declared and again each time its owner is (re)mounted; what it added goes with the slot, and with your plugin on unload. A plugin may therefore mount before the plugin that declares its slot.

A component reaches the services of the plugin that registered it with `useCtx()` (call it while initialising): `const { daemon } = useCtx();`. The plugin must list the service in `inject`.

## Keys and commands

`keymap.bind("ctrl-s", (event) => …)`: a chord is `ctrl-alt-meta-shift-` modifiers (in that order) and the lower-case key: `ctrl-s`, `shift-tab`, `escape`, `f6`. The window hands a handler every key the focused input or panel did not use. The newest binding of a chord runs first; return `false` to say the key was not yours (it passes on, and the input still gets it), anything else keeps the input from typing it. `keymap.observe(fn)` hears every key before the bindings. A chord matches exactly (`ctrl-shift-s` is not `ctrl-s`).

`commands.register({ name, desc, run, args?, needsArgs? })` adds an entry to the `/` menu that the window runs itself; without a `run` the text goes to the daemon. Entries list in registration order.

## Host services

| Service    | What                                                                                                          |
| ---------- | ------------------------------------------------------------------------------------------------------------- |
| `slots`    | `add`, `inject` (above)                                                                                       |
| `daemon`   | the session, chat, palette, status, `open`, `send`, `newSession`, `onRecord`… (wraps `state/suite.svelte.ts`) |
| `git`      | `status` (reactive), `branch`, `diff`, `places` (host commands)                                               |
| `github`   | `repo`, `issues`, `prs`, `issue`, `pr`, `prDiff`, … (read-only host commands)                                 |
| `keymap`   | `bind`, `observe`                                                                                             |
| `commands` | `register`, `list`                                                                                            |

The old singletons (`suite`, `ui`, `composer`) are still the stores behind them, and panes not converted yet read them directly. A converted pane reads the service and keeps its own state in its folder.

## State, and the `$state` trap

**Never put `$state` fields on a `Service` subclass.** Svelte compiles them to private fields, and `isolate`/`intercept` re-create a service with `Object.create()`, which then throws on the inherited accessor. Keep reactive state in a `.svelte.ts` store and let the Service delegate to it (see `services/daemon.ts`, `commands.svelte.ts`).

A pane's own store lives in its folder (`plugins/sessions/state.svelte.ts`). A store that needs `$effect`s exposes `start(daemon): () => void` built on `$effect.root`, and the plugin runs it with `ctx.effect(() => store.start(daemon))` so unloading stops it. Other code that needs the state (the composer reads `sessions.focused`) imports the store; once more than one consumer needs it, put a service in front.

Keep logic that tests should reach in plain `.ts` files (`bun test` cannot compile runes): `slots/registry.ts`, `keymap.ts` and `lib/*.ts` are tested that way.

## Things that bite

- Inside a service method, `this.ctx` resolves services through the service's own injects, and only effects belong to the caller. To get the caller's context (as `slots` does for `useCtx`), read `this[symbols.caller]`.
- `await ctx.plugin(p)` on a plugin whose `inject` is not satisfied does not wait: it stays `PENDING`. `boot` reports those at the end.
- A failed plugin does not block the window: `boot` rejects after mounting the rest, and the host logs it (`errors.log` without `--dev`).
- Types and `bun test` read extension-system's built `lib/` (`bun run build` there); the bundle reads its `src/`.

## Moving a legacy pane

1. A folder `plugins/<name>/` with `index.ts`; `git mv` its component, store and pure logic in.
2. Replace `suite.*` with `useCtx().daemon`, `ui.*` flags with a store of its own, and registration in `plugins/legacy/*` with `slots.inject`.
3. Move its keys from `legacy/shortcuts.ts` to `keymap.bind`, its slash commands to `commands.register`.
4. If other panes sit inside it, declare slots for them with `slots: { … }` and move them in as entries.
5. List it in `plugins/index.ts`, run `bun test`, `make typecheck`, `bunx oxlint --type-aware --deny-warnings`, build, and look at it with `apps/gui/scripts/shot.sh` (`SCENE=side`) next to the old layout.
