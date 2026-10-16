// One clock for the ages and elapsed times on screen, ticking only while something shows one.
const INTERVAL_MS = 1000;

class Clock {
  /** Unix milliseconds, as of the last tick. */
  now = $state(Date.now());
  private users = 0;
  private timer: number | undefined;

  /** Something that shows a time appeared; returns the call that says it went away. */
  join(): () => void {
    this.users += 1;
    this.now = Date.now();
    if (this.timer === undefined) {
      this.timer = setInterval(() => (this.now = Date.now()), INTERVAL_MS);
    }
    return () => {
      this.users -= 1;
      if (this.users === 0 && this.timer !== undefined) {
        clearInterval(this.timer);
        this.timer = undefined;
      }
    };
  }
}

export const clock = new Clock();
