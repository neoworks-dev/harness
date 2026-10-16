import { afterEach } from "bun:test";

/**
 * Call once at the top of a test file. Returns a function that registers something to undo
 * after each test (stop a daemon, close a client); they run last registered first.
 */
export function cleanupAfterEach(): (undo: () => Promise<unknown> | void) => void {
  const undos: (() => Promise<unknown> | void)[] = [];
  afterEach(async () => {
    for (const undo of undos.splice(0).reverse()) await undo();
  });
  return (undo) => {
    undos.push(undo);
  };
}
