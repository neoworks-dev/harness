// The host services, mounted before any plugin: what plugins build on. Each wraps a store or a
// host command of the window and publishes it under its name on `ctx`.
import type { Plugin } from "@neoworks/extension-system";

export const services: Plugin[] = [];
