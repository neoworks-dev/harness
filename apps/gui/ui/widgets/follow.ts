// Whether a list that grows at its end (the chat, the terminal panel) follows it. The list's own
// `stickToEnd` lets go whenever the host reports it short of the end, which a list that just
// opened is when its rows come out taller than guessed: then it never follows. This decides by
// what the user did: it follows from the start, lets go when they scroll up, and takes hold again
// when they come back to the end. Plain, tested without a window.

/** How close to the end, in pixels, counts as at it. */
const NEAR_END = 4;

export class Follow {
  following = true;
  private lastY = 0;

  /** A scroll report. Content that shrinks moves `y` up too, but to the end, so that still follows. */
  scrolled(y: number, maxY: number): void {
    if (maxY - y <= NEAR_END) this.following = true;
    else if (y < this.lastY - 1) this.following = false;
    this.lastY = y;
  }

  /** Another session or a fresh start: follow again. */
  reset(): void {
    this.following = true;
    this.lastY = 0;
  }
}
