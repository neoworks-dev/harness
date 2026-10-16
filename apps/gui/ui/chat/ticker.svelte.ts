// One clock for every spinner on screen, running only while one is mounted.
const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const INTERVAL_MS = 90;

class Ticker {
  private tick = $state(0);
  private users = 0;
  private timer: number | undefined;

  /** The spinner's current glyph. */
  get frame(): string {
    return FRAMES[this.tick % FRAMES.length] ?? "";
  }

  /** A spinner appeared; returns the call that says it went away. */
  join(): () => void {
    this.users += 1;
    if (this.timer === undefined) this.timer = setInterval(() => (this.tick += 1), INTERVAL_MS);
    return () => {
      this.users -= 1;
      if (this.users === 0 && this.timer !== undefined) {
        clearInterval(this.timer);
        this.timer = undefined;
      }
    };
  }
}

export const ticker = new Ticker();
