import { startExtensionClient } from "@neoworks/neoshell-sdk";
import Agents from "./Agents.svelte";

startExtensionClient("dev.neoworks.agents", { agents: Agents });
