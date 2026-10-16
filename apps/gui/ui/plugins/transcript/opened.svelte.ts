// Which tool results the user opened by clicking their header. Kept here and not in the row:
// the list builds rows only near the window, so a row's own state is lost when it scrolls away.
class Opened {
  private refs = $state.raw<ReadonlySet<string>>(new Set());

  has(ref: string): boolean {
    return this.refs.has(ref);
  }

  toggle(ref: string): void {
    const next = new Set(this.refs);
    if (!next.delete(ref)) next.add(ref);
    this.refs = next;
  }
}

export const opened = new Opened();
