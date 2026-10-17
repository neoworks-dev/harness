# The window's plugin kernel

Every pane of the window is a plugin on [`@neoworks/extension-system`](../../../../../extension-system/README.md) (read its `skills/extension-system/SKILL.md` first: `ctx.effect`, `inject`, the fiber lifecycle). Plugins are compiled into the bundle; nothing is loaded at run time, and external apps still only get templates.

```
kernel/          the root Context, boot, host services, slots, keymap, commands
plugins/         one folder per pane; index.ts lists them in mount order
widgets/         small shared pieces that belong to no pane (the spinner, the clock)
lib/             pure logic more than one pane uses (palette, diff, commands, unified diffs, git status, comments)
```

A pane reaches another only through a service or a slot, never by importing its modules. Pure logic that only one pane uses lives in its folder.

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

`plugins/sessions` (sidebar, keys, state) and `plugins/statusline` (declares a slot another plugin fills) are the worked examples; `plugins/transcript` (a keyed slot with a renderer per kind, a service) is the larger one.

### The window's panes

| Plugin       | Fills                                                       | Declares                                                                | Provides     |
| ------------ | ----------------------------------------------------------- | ----------------------------------------------------------------------- | ------------ |
| `shell`      | `root`                                                      | `shell.left`, `.main` (chain), `.bottom`, `.right`, `.card`, `.overlay` |              |
| `sessions`   | `shell.left`                                                |                                                                         |              |
| `statusline` | `shell.bottom` (30)                                         | `statusline.right`                                                      |              |
| `transcript` | `shell.main` (100), `chat.item`, `.header`                  | `chat.row`, `chat.item` (keyed), `chat.item.header`                     | `transcript` |
| `composer`   | `shell.bottom` (10), `statusline.right`                     | `composer.left`, `composer.right`                                       |              |
| `agents`     | `shell.bottom` (20)                                         |                                                                         |              |
| `agentview`  | `shell.main` (20)                                           |                                                                         | `agentview`  |
| `sidepanel`  | `shell.right`                                               | `sidepanel.section`                                                     | `sidepanel`  |
| `plan`       | `sidepanel.section` (10)                                    |                                                                         |              |
| `files`      | `sidepanel.section` (20)                                    |                                                                         |              |
| `github`     | `shell.main` (10)                                           |                                                                         |              |
| `comments`   |                                                             |                                                                         | `comments`   |
| `viewer`     | `shell.right` (-10), `shell.main` (5), `shell.toolbar` (15) |                                                                         | `viewer`     |
| `workspace`  | `shell.card`, `shell.toolbar` (30)                          |                                                                         |              |

A plugin lists the services it uses in `inject` and waits for them, so a plugin that provides one is listed before those that use it in `plugins/index.ts` (the order also decides ties in a list slot and the order of the `/` menu). A plugin provides a service by mounting a `Service` of its own: `ctx.plugin(ViewerService)`; it unloads with the plugin.

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

`keymap.bind("ctrl-s", (event) => …)`: a chord is `ctrl-alt-meta-shift-` modifiers (in that order) and the lower-case key: `ctrl-s`, `shift-tab`, `escape`, `f6`. The window hands a handler every key the focused input or panel did not use. The newest binding of a chord runs first, unless another has a higher `priority` (`keymap.bind("escape", run, { priority: 1 })`, for an overlay that covers the window); return `false` to say the key was not yours (it passes on, and the input still gets it), anything else keeps the input from typing it. `keymap.observe(fn)` hears every key before the bindings. A chord matches exactly (`ctrl-shift-s` is not `ctrl-s`).

`commands.register({ name, desc, run, args?, needsArgs? })` adds an entry to the `/` menu that the window runs itself; without a `run` the text goes to the daemon. Entries list in registration order.

## Host services

| Service    | What                                                                                                                     |
| ---------- | ------------------------------------------------------------------------------------------------------------------------ |
| `slots`    | `add`, `inject` (above)                                                                                                  |
| `daemon`   | the session, chat, palette, status, `open`, `send`, `action`, `newSession`, `onRecord`… (wraps `kernel/suite.svelte.ts`) |
| `git`      | `branch`, `status`, `diff`, `places` (host commands)                                                                     |
| `github`   | `repo`, `issues`, `prs`, `issue`, `pr`, `prDiff`, … (read-only host commands)                                            |
| `keymap`   | `bind`, `observe`                                                                                                        |
| `keyboard` | `claim(held)`, `taken`: a pane that wants the keyboard claims it, and the input stays away while one is held             |
| `commands` | `register`, `list`                                                                                                       |

What the panes offer one another is in the table above: `viewer.showChanges(path)`, `viewer.showDiff(diff)` and `viewer.openFile(path)`, `comments.add(comment)` and `comments.take(text)`, `transcript.visible(items)` and `transcript.stub(ref)`, `agentview.open(agent)`, `sidepanel.claim(has)` and `sidepanel.cover(when)`. Each is in the plugin's `service.ts`.

## State, and the `$state` trap

**Never put `$state` fields on a `Service` subclass.** Svelte compiles them to private fields, and `isolate`/`intercept` re-create a service with `Object.create()`, which then throws on the inherited accessor. Keep reactive state in a `.svelte.ts` store and let the Service delegate to it (see `services/daemon.ts`, `commands.svelte.ts`).

A pane's own store lives in its folder (`plugins/sessions/state.svelte.ts`). A store that needs `$effect`s or services exposes `start(daemon): () => void` built on `$effect.root`, and the plugin runs it with `ctx.effect(() => store.start(daemon))` so unloading stops it. What another pane needs of the state goes through a service in front of the store, never an import: the composer asks `keyboard.taken`, not the sessions store.

Two helpers cover what several panes repeat. `Folded` (`kernel/folded.svelte.ts`) keeps state folded from the shown session's events, restarted on a replay (the agents, the plan and the files each keep one). `Claims` (`kernel/claims.svelte.ts`) is a set of reactive yes/no claims, behind the `keyboard` and `sidepanel` services.

Keep logic that tests should reach in plain `.ts` files (`bun test` cannot compile runes): `slots/registry.ts`, `keymap.ts` and `lib/*.ts` are tested that way.

## Things that bite

- Inside a service method, `this.ctx` resolves services through the service's own injects, and only effects belong to the caller. To get the caller's context (as `slots` does for `useCtx`), read `this[symbols.caller]`.
- `await ctx.plugin(p)` on a plugin whose `inject` is not satisfied does not wait: it stays `PENDING`. `boot` reports those at the end.
- A failed plugin does not block the window: `boot` rejects after mounting the rest, and the host logs it (`errors.log` without `--dev`).
- Types and `bun test` read extension-system's built `lib/` (`bun run build` there); the bundle reads its `src/`.

## Adding a pane

1. A folder `plugins/<name>/` with `index.ts`, its components, its store and the pure logic only it uses.
2. Components read the services with `useCtx()` (`daemon.palette`, …) and keep their state in a store of the folder; a flag two panes would share is a service one of them provides.
3. Register into the slot it belongs in with `slots.inject`, its keys with `keymap.bind`, its slash commands with `commands.register`.
4. If other panes sit inside it, declare slots for them with `slots: { … }` and let them register as entries; put the declarations in `slots.ts`.
5. List it in `plugins/index.ts` (after the plugins whose services it uses), run `bun test`, `make typecheck`, `bunx oxlint --type-aware --deny-warnings`, build, and look at it with `apps/gui/scripts/shot.sh` (`SCENE=side`).
