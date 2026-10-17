// What each kind of word is drawn with: the palette's colour as the text style of a run. One set of
// styles per palette, so a line of code does not build its own.
import type { Style } from "@neoworks/svelte-gpui/ui";
import type { Syntax } from "../../lib/palette.ts";
import type { Kind } from "./syntax.ts";

const made = new WeakMap<Syntax, Readonly<Record<Kind, Style>>>();

/** The style of a run of each kind. */
export function kindStyles(syntax: Syntax): Readonly<Record<Kind, Style>> {
  const known = made.get(syntax);
  if (known !== undefined) return known;
  const styles: Record<Kind, Style> = {
    keyword: { color: syntax.keyword },
    string: { color: syntax.string },
    number: { color: syntax.number },
    comment: { color: syntax.comment },
    function: { color: syntax.function },
    type: { color: syntax.type },
    constant: { color: syntax.constant },
    tag: { color: syntax.tag },
    attr: { color: syntax.attr },
    property: { color: syntax.property },
  };
  made.set(syntax, styles);
  return styles;
}
