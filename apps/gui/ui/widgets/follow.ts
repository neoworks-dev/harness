// Whether a list that grows at its end (the chat, the terminal panel) follows it. The list's own
// `stickToEnd` lets go whenever the host reports it short of the end, which a list that just
// opened is when its rows come out taller than guessed: then it never follows. This decides by
// what the user did: it follows from the start, lets go when they scroll up, and takes hold again
// when they come back to the end. Only the user's own scrolling lets go: a list that moves up
// because its rows reflowed (a panel opened beside it, the window was resized) still follows.
// Plain, tested without a window.

/** How close to the end, in pixels, counts as at it. */
const NEAR_END = 4;
/** How long after the user's wheel or press a scroll is theirs. */
const INPUT_MS = 600;

export class Follow {
  following = true;
  private lastY = 0;
  private inputAt = Number.NEGATIVE_INFINITY;

  /** The user turned the wheel over the list or pressed it (its scrollbar). An upward turn lets go at once. */
  touched(now: number, dy = 0): void {
    this.inputAt = now;
    if (dy < 0) this.following = false;
  }

  /**
   * A scroll report. Back at the end it follows again; moved up right after the user's input it
   * lets go. Content that shrinks moves `y` up too, but to the end, so that still follows.
   */
  scrolled(y: number, maxY: number, now: number): void {
    if (maxY - y <= NEAR_END) this.following = true;
    else if (y < this.lastY - 1 && now - this.inputAt <= INPUT_MS) this.following = false;
    this.lastY = y;
  }

  /** Another session or a fresh start: follow again. */
  reset(): void {
    this.following = true;
    this.lastY = 0;
    this.inputAt = Number.NEGATIVE_INFINITY;
  }
}
