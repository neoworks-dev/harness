// Installs the widget for this user: the extension folder neoshell reads at startup, and the
// D-Bus service file that starts this app when the shell first asks for its data.
import { copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const EXTENSION_ID = "dev.neoworks.agents";
const DBUS_NAME = "dev.neoworks.Neo.Agents";

const here = (path: string): string => fileURLToPath(new URL(path, import.meta.url));
const dataHome = process.env.XDG_DATA_HOME || join(homedir(), ".local", "share");

const extension = join(dataHome, "neoshell", "extensions", EXTENSION_ID);
mkdirSync(extension, { recursive: true });
copyFileSync(here("./dist/manifest.json"), join(extension, "manifest.json"));
copyFileSync(here("./dist/client.js"), join(extension, "client.js"));

const services = join(dataHome, "dbus-1", "services");
mkdirSync(services, { recursive: true });
writeFileSync(
  join(services, `${DBUS_NAME}.service`),
  `[D-BUS Service]\nName=${DBUS_NAME}\nExec=${process.execPath} ${here("./src/main.ts")}\n`,
);

process.stdout.write(
  `installed ${extension} and the ${DBUS_NAME} service; restart neoshell to load it\n`,
);
