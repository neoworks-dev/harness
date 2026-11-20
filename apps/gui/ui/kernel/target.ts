// Where the window runs: its own window, or a browser tab (main.web.ts says so before anything
// else is loaded). For the few defaults that differ, such as the chat's size.
export const target: {
  web: boolean;
  /** A browser tab opened on this session (`?session=…`), in place of the one startup picks. */
  session?: string;
  /** The tab is a column beside another session's (`?column`): it has no sidebar of its own. */
  column: boolean;
  /** Opens a session in a column of its own beside this one; only where columns can be had (a browser tab). */
  openBeside?: (session: string) => void;
} = { web: false, column: false };
