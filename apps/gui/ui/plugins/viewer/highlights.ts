// The syntax highlighting of what the viewer shows, per file and kept for as long as the file's
// lines are: a diff's lines are two texts woven together (the old one with its removed lines, the
// new one with its added lines, the unchanged ones in both), and each is scanned as the code it is.
import type { DiffLine } from "@neoworks-dev/neo-protocol/types";
import { Highlighter, languageOf, type Token } from "./syntax.ts";

/** What highlights the lines of one diff or file. */
export interface Lines {
  /** The runs of line `index`, or undefined when the file is not code the highlighter knows. */
  tokens(index: number): Token[] | undefined;
}

/** Files scanned ahead, one slice at a time, while the window is idle. */
const queue: Highlighter[] = [];
let running = false;

/** The lines scanned in one go before the window gets the thread back. */
const SLICE = 250;

function warmNext(): void {
  const light = queue[0];
  if (light === undefined) {
    running = false;
    return;
  }
  light.warm(SLICE);
  if (!light.pending) queue.shift();
  setTimeout(warmNext, 8);
}

/** Has the file scanned ahead of the window, so a jump far into it finds its states ready. */
function warmLater(light: Highlighter): void {
  queue.push(light);
  if (running) return;
  running = true;
  setTimeout(warmNext, 50);
}

class Plain implements Lines {
  private light?: Highlighter;

  constructor(
    private readonly language: string,
    private readonly lines: readonly string[],
  ) {}

  tokens(index: number): Token[] {
    if (this.light === undefined) {
      this.light = new Highlighter(this.language, this.lines);
      warmLater(this.light);
    }
    return this.light.tokens(index);
  }
}

class Woven implements Lines {
  private oldLight?: Highlighter;
  private newLight?: Highlighter;
  /** For each line: whether it is in the old text, and where it is in the text it is read from. */
  private readonly places: { old: boolean; at: number }[] = [];
  private readonly oldLines: string[] = [];
  private readonly newLines: string[] = [];

  constructor(
    private readonly language: string,
    lines: readonly DiffLine[],
  ) {
    for (const line of lines) {
      if (line.kind === "hunk") {
        // What is between two hunks is not here: each text goes on from a blank line.
        this.oldLines.push("");
        this.newLines.push("");
        this.places.push({ old: false, at: this.newLines.length - 1 });
      } else if (line.kind === "remove") {
        this.oldLines.push(line.text);
        this.places.push({ old: true, at: this.oldLines.length - 1 });
      } else {
        if (line.kind === "context") this.oldLines.push(line.text);
        this.newLines.push(line.text);
        this.places.push({ old: false, at: this.newLines.length - 1 });
      }
    }
  }

  tokens(index: number): Token[] | undefined {
    const place = this.places[index];
    if (place === undefined) return undefined;
    if (place.old) {
      this.oldLight ??= this.start(this.oldLines);
      return this.oldLight.tokens(place.at);
    }
    this.newLight ??= this.start(this.newLines);
    return this.newLight.tokens(place.at);
  }

  private start(lines: readonly string[]): Highlighter {
    const light = new Highlighter(this.language, lines);
    warmLater(light);
    return light;
  }
}

const diffs = new WeakMap<readonly DiffLine[], Lines | null>();

/** The highlighting of a file's diff, or undefined for a file that is not code. */
export function forDiff(path: string, lines: readonly DiffLine[]): Lines | undefined {
  const known = diffs.get(lines);
  if (known !== undefined) return known ?? undefined;
  const language = languageOf(path);
  const made = language === undefined ? null : new Woven(language, lines);
  diffs.set(lines, made);
  return made ?? undefined;
}

const files = new WeakMap<readonly string[], Lines | null>();

/** The highlighting of a file's lines, or undefined for a file that is not code. */
export function forFile(path: string, lines: readonly string[]): Lines | undefined {
  const known = files.get(lines);
  if (known !== undefined) return known ?? undefined;
  const language = languageOf(path);
  const made = language === undefined ? null : new Plain(language, lines);
  files.set(lines, made);
  return made ?? undefined;
}
