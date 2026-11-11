// Whether the terminal panel is shown, and how wide it is, for the window's lifetime.

/** The panel is never narrower than this. */
export const MIN_WIDTH = 360;

class Terminal {
  shown = $state(false);
  width = $state(520);

  toggle(): void {
    this.shown = !this.shown;
  }
}

export const terminal = new Terminal();
