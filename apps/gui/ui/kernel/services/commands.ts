// The `commands` service: the `/` menu's entries. A plugin registers one with a `run`, which the
// window calls when the user picks it or types it.
import { Service, type Context } from "@neoworks/extension-system";
import type { Command } from "../../lib/commands.ts";
import { commandStore } from "../commands.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    commands: CommandsService;
  }
}

/** A command as a plugin writes it: only the name, what it does and `run` are needed. */
export type NewCommand = Pick<Command, "name" | "desc" | "run"> & Partial<Command>;

export class CommandsService extends Service {
  constructor(ctx: Context) {
    super(ctx, "commands");
  }

  /** Every entry the menu lists now. */
  get list(): readonly Command[] {
    return commandStore.list;
  }

  /** Adds an entry to the menu. */
  register(command: NewCommand): () => void {
    const dispose = this.ctx.effect(
      () => commandStore.add({ args: "", needsArgs: false, ...command }),
      `commands.register(${command.name})`,
    );
    return () => void dispose();
  }
}
