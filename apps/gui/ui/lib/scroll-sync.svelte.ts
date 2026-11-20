// The chat and the terminal panel keep each other in step. Scrolling the chat up brings the
// terminal to the newest command at or above the chat's last row, and back at the chat's end the
// terminal follows its end again; scrolling the terminal brings that command's row into view in
// the chat. Each side asks the other here; a scroll it was asked for is not passed back, so the
// two never chase each other.

/** How long after a scroll it was asked for a side's own scroll reports are its echo. */
const ECHO_MS = 400;

class ScrollSync {
  /** The terminal is asked to show this command (its item's ref), or its end. */
  terminal = $state.raw<{ ref: string | "end"; seq: number }>();
  /** The chat is asked to show the row of this command. */
  chat = $state.raw<{ ref: string; seq: number }>();
  private seq = 0;
  private quiet = { chat: 0, terminal: 0 };

  /** The chat's user scrolled: the terminal follows to `ref`, or to its end. */
  fromChat(ref: string | "end"): void {
    if (Date.now() < this.quiet.chat) return;
    this.quiet.terminal = Date.now() + ECHO_MS;
    this.terminal = { ref, seq: (this.seq += 1) };
  }

  /** The terminal's user scrolled: the chat shows `ref`'s row. */
  fromTerminal(ref: string): void {
    if (Date.now() < this.quiet.terminal) return;
    this.quiet.chat = Date.now() + ECHO_MS;
    this.chat = { ref, seq: (this.seq += 1) };
  }
}

export const scrollSync = new ScrollSync();
