import { FiberState, type Context, type Plugin } from "@neoworks/extension-system";

/**
 * Mounts the host services, then the plugins, each in the order listed, so what a plugin
 * registers into lands where the list says. A host service that fails stops the boot. A plugin
 * that fails is reported and left out, so one broken pane does not blank the window; one that is
 * still waiting for a service when all are mounted is reported too.
 */
export async function boot(ctx: Context, services: Plugin[], plugins: Plugin[]): Promise<void> {
  for (const service of services) await ctx.plugin(service);
  const waiting = [];
  for (const plugin of plugins) {
    const fiber = ctx.plugin(plugin);
    await fiber.then(undefined, (error: unknown) =>
      console.error(`plugin ${fiber.name} failed`, error),
    );
    // Its siblings may still provide what it needs, so this is reported once all are mounted.
    if (fiber.state === FiberState.PENDING) waiting.push(fiber);
  }
  for (const fiber of waiting) {
    if (fiber.state === FiberState.PENDING)
      console.error(`plugin ${fiber.name} is waiting for a service`);
  }
}
