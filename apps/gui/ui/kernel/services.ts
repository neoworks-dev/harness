// The host services, mounted before any plugin: what plugins build on. Each wraps a store or a
// host command of the window and publishes it under its name on `ctx`.
import type { Plugin } from "@neoworks/extension-system";
import { CommandsService } from "./services/commands.ts";
import { DaemonService } from "./services/daemon.ts";
import { GitService } from "./services/git.ts";
import { GithubService } from "./services/github.ts";
import { KeyboardService } from "./services/keyboard.ts";
import { KeymapService } from "./services/keymap.ts";
import { SlotsService } from "./slots/service.ts";

export const services: Plugin[] = [
  SlotsService,
  DaemonService,
  GitService,
  GithubService,
  KeymapService,
  KeyboardService,
  CommandsService,
];
