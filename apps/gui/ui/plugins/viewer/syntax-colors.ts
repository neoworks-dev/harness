// What each kind of word is drawn with: the palette's colour as the text style of a run. One set of
// styles per palette, so a line of code does not build its own.
import type { Style } from "@neoworks/svelte-gpui/ui";
import type { Syntax } from "../../lib/palette.ts";
import type { Kind } from "./syntax.ts";

/** The style of a run of each kind, and of one that is not coloured. */
export type Styles = Readonly<Record<Kind | "plain", Style>>;

const made = new WeakMap<Syntax, Map<string, Styles>>();

/**
 * The style of a run of each kind; with `background`, of the runs that are marked as what changed
 * in a line. They are made once per palette and background, not per line.
 */
export function kindStyles(syntax: Syntax, background = ""): Styles {
  const sets = made.get(syntax) ?? new Map<string, Styles>();
  made.set(syntax, sets);
  const known = sets.get(background);
  if (known !== undefined) return known;
  const on = (color?: string): Style => {
    const style: Style = {};
    if (color !== undefined) style.color = color;
    if (background !== "") style.background = background;
    return style;
  };
  const styles: Styles = {
    plain: on(),
    keyword: on(syntax.keyword),
    string: on(syntax.string),
    number: on(syntax.number),
    comment: on(syntax.comment),
    function: on(syntax.function),
    type: on(syntax.type),
    constant: on(syntax.constant),
    tag: on(syntax.tag),
    attr: on(syntax.attr),
    property: on(syntax.property),
  };
  sets.set(background, styles);
  return styles;
}
