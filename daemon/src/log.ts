// Minimal stderr logger (the daemon runs as a user service; journald/launchd capture stderr).
export interface Logger {
  info(msg: string): void;
  warn(msg: string): void;
  error(msg: string): void;
}

export function createLogger(
  write: (line: string) => void = (l) => process.stderr.write(l),
): Logger {
  const at = (level: string) => (msg: string) =>
    write(`${new Date().toISOString()} ${level} ${msg}\n`);
  return { info: at("info"), warn: at("warn"), error: at("error") };
}

export const silentLogger: Logger = { info: () => {}, warn: () => {}, error: () => {} };
