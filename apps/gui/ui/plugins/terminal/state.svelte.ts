// Whether the terminal panel is shown, and how wide it is, for the window's lifetime.

/** The panel is never narrower than this. */
export const MIN_WIDTH = 360;

class Terminal {
  shown = $state(false);
  width = $state(520);
  /** The background command shown on its own (its run's id); undefined shows them all. */
  view = $state<string>();

  toggle(): void {
    this.shown = !this.shown;
    if (!this.shown) this.view = undefined;
  }

  /** Opens the panel on one background command. */
  showJob(id: string): void {
    this.view = id;
    this.shown = true;
  }
}

export const terminal = new Terminal();
