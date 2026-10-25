// Theme → CSS custom properties, and the stylesheet that uses only those. The daemon's Theme
// (protocol/themes/default.json) is the single source of colors, spacing and type sizes: nothing
// in the stylesheet below hard-codes them, so a new theme restyles the chat without new CSS.
import type { TextStyle, Theme } from "@neoworks-dev/neo-protocol/types";

/** Font stacks for a TextStyle's `mono` flag (the theme only says mono or not). */
const FONT_SANS = 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif';
const FONT_MONO = 'ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace';

function textVariables(name: string, style: TextStyle): Record<string, string> {
  let font = "var(--neo-font-sans)";
  if (style.mono) font = "var(--neo-font-mono)";
  return {
    [`--neo-${name}-size`]: `${style.size}px`,
    [`--neo-${name}-weight`]: String(style.weight),
    [`--neo-${name}-font`]: font,
  };
}

/** The custom properties a theme sets on the chat container. */
export function themeVariables(theme: Theme): Record<string, string> {
  const { colors, spacing, text } = theme;
  return {
    "--neo-fg": colors.fg,
    "--neo-bg": colors.bg,
    "--neo-surface": colors.surface,
    "--neo-border": colors.border,
    "--neo-accent": colors.accent,
    "--neo-muted": colors.muted,
    "--neo-success": colors.success,
    "--neo-danger": colors.danger,
    "--neo-space-s": `${spacing.s}px`,
    "--neo-space-m": `${spacing.m}px`,
    "--neo-space-l": `${spacing.l}px`,
    "--neo-font-sans": FONT_SANS,
    "--neo-font-mono": FONT_MONO,
    ...textVariables("title", text.title),
    ...textVariables("body", text.body),
    ...textVariables("code", text.code),
  };
}

/** Selector prefix of every rule: the container mountChat was given. */
export const CHAT_CLASS = "neo-chat";

const TONES = ["accent", "muted", "success", "danger"];

function toneRules(selector: (tone: string) => string, property: string): string {
  return TONES.map((tone) => `${selector(tone)} { ${property}: var(--neo-${tone}); }`).join("\n");
}

export const chatStyles = `
.${CHAT_CLASS} {
  display: flex;
  flex-direction: column;
  gap: var(--neo-space-m);
  box-sizing: border-box;
  padding: var(--neo-space-l);
  overflow-y: auto;
  color: var(--neo-fg);
  background: var(--neo-bg);
  font-family: var(--neo-body-font);
  font-size: var(--neo-body-size);
  font-weight: var(--neo-body-weight);
  line-height: 1.5;
}
.${CHAT_CLASS} *, .${CHAT_CLASS} *::before, .${CHAT_CLASS} *::after { box-sizing: border-box; }
.${CHAT_CLASS} :focus-visible { outline: 2px solid var(--neo-accent); outline-offset: 2px; }

.neo-item { max-width: 100%; }
.neo-item[tabindex] { border-radius: 6px; }
.neo-item-header { color: var(--neo-muted); font-size: 0.9em; margin-bottom: var(--neo-space-s); }
.neo-item[data-kind="user"] {
  align-self: flex-end;
  max-width: 85%;
  padding: var(--neo-space-m) var(--neo-space-l);
  border: 1px solid var(--neo-border);
  border-radius: 12px 12px 2px 12px;
  background: var(--neo-surface);
}
.neo-item[data-kind="tool"] > .neo-item-body { margin-left: var(--neo-space-l); }
.neo-item[data-kind="notice"] { color: var(--neo-muted); font-size: 0.9em; }
.neo-item[data-kind="notice"] .neo-text > p { margin: 0; }
.neo-item[data-kind="thought"] > summary { cursor: pointer; margin-bottom: 0; }
.neo-item[data-kind="thought"][open] > summary { margin-bottom: var(--neo-space-s); }
.neo-item[data-kind="thought"] > .neo-item-body { color: var(--neo-muted); font-style: italic; }

.neo-state { display: inline-block; }
.neo-state[data-state="pending"] { color: var(--neo-muted); }
.neo-state[data-state="running"] { color: var(--neo-accent); animation: neo-spin 1.2s linear infinite; }
.neo-state[data-state="done"] { color: var(--neo-success); }
.neo-state[data-state="failed"] { color: var(--neo-danger); }
@keyframes neo-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .neo-state[data-state="running"] { animation: none; } }

.neo-card {
  padding: var(--neo-space-m) var(--neo-space-l);
  border: 1px solid var(--neo-border);
  border-radius: 8px;
  background: var(--neo-surface);
  display: flex;
  flex-direction: column;
  gap: var(--neo-space-s);
  min-width: 0;
}
.neo-card-title { margin: 0; font-size: var(--neo-title-size); font-weight: var(--neo-title-weight); }
${toneRules((tone) => `.neo-card[data-tone="${tone}"]`, "border-color")}
${toneRules((tone) => `.neo-card[data-tone="${tone}"] > .neo-card-title`, "color")}

.neo-row, .neo-stack { display: flex; gap: var(--neo-space-s); min-width: 0; }
.neo-row { flex-direction: row; flex-wrap: wrap; align-items: flex-start; }
.neo-stack { flex-direction: column; }
.neo-row > *, .neo-stack > * { min-width: 0; }
.neo-row[data-gap="s"], .neo-stack[data-gap="s"] { gap: var(--neo-space-s); }
.neo-row[data-gap="m"], .neo-stack[data-gap="m"] { gap: var(--neo-space-m); }
.neo-row[data-gap="l"], .neo-stack[data-gap="l"] { gap: var(--neo-space-l); }

.neo-text > :first-child { margin-top: 0; }
.neo-text > :last-child { margin-bottom: 0; }
.neo-text p, .neo-text ul, .neo-text ol, .neo-text blockquote, .neo-text pre {
  margin: 0 0 var(--neo-space-s);
}
.neo-text[data-role="title"] { font-size: var(--neo-title-size); font-weight: var(--neo-title-weight); font-family: var(--neo-title-font); }
.neo-text[data-role="code"] { font-size: var(--neo-code-size); font-weight: var(--neo-code-weight); font-family: var(--neo-code-font); }
${toneRules((tone) => `.neo-text[data-tone="${tone}"]`, "color")}
.neo-heading { margin: var(--neo-space-m) 0 var(--neo-space-s); font-size: var(--neo-body-size); font-weight: var(--neo-title-weight); }
h1.neo-heading, h2.neo-heading { font-size: var(--neo-title-size); }
.neo-list { padding-left: calc(var(--neo-space-l) * 1.5); }
.neo-list .neo-list { margin-bottom: 0; }
.neo-list li > p { margin: 0; }
.neo-task { list-style: none; margin-left: calc(var(--neo-space-l) * -1.5); padding-left: 0; }
.neo-checkbox { margin: 0 var(--neo-space-s) 0 0; vertical-align: middle; accent-color: var(--neo-accent); }
.neo-task > p { display: inline; }
.neo-quote { padding-left: var(--neo-space-m); border-left: 3px solid var(--neo-border); color: var(--neo-muted); margin-left: 0; margin-right: 0; }
.neo-rule { border: 0; border-top: 1px solid var(--neo-border); margin: var(--neo-space-m) 0; }
.neo-link { color: var(--neo-accent); text-decoration: underline; }
.neo-inline-code, .neo-code {
  font-family: var(--neo-code-font);
  font-size: var(--neo-code-size);
  font-weight: var(--neo-code-weight);
}
.neo-inline-code { padding: 0 0.3em; border-radius: 4px; background: var(--neo-surface); }
.neo-code {
  margin: 0;
  padding: var(--neo-space-m);
  overflow-x: auto;
  white-space: pre;
  border-radius: 6px;
  background: var(--neo-surface);
}
.neo-card .neo-code, .neo-user .neo-code { background: var(--neo-bg); }
.neo-code > code { font: inherit; }

.neo-image { margin: 0; }
.neo-image img { max-width: 100%; height: auto; border-radius: 6px; }
.neo-image figcaption { color: var(--neo-muted); font-size: 0.9em; }
.neo-image-fallback, .neo-unsupported {
  display: inline-block;
  max-width: 100%;
  padding: var(--neo-space-s) var(--neo-space-m);
  border: 1px dashed var(--neo-border);
  border-radius: 6px;
  color: var(--neo-muted);
  font-family: var(--neo-code-font);
  font-size: var(--neo-code-size);
  overflow-wrap: anywhere;
}

.neo-file {
  display: inline-block;
  padding: 0 var(--neo-space-m);
  border: 1px solid var(--neo-border);
  border-radius: 999px;
  background: var(--neo-surface);
}

.neo-diff { margin: 0; border: 1px solid var(--neo-border); border-radius: 6px; overflow: hidden; }
.neo-diff-path { padding: var(--neo-space-s) var(--neo-space-m); color: var(--neo-muted); background: var(--neo-surface); border-bottom: 1px solid var(--neo-border); font-family: var(--neo-code-font); font-size: var(--neo-code-size); }
.neo-diff-body { margin: 0; overflow-x: auto; font-family: var(--neo-code-font); font-size: var(--neo-code-size); }
.neo-diff-line { display: flex; white-space: pre; padding-right: var(--neo-space-m); }
.neo-diff-num { flex: none; min-width: 3.5ch; padding: 0 var(--neo-space-s); text-align: right; color: var(--neo-muted); user-select: none; }
.neo-diff-sign { flex: none; width: 2ch; text-align: center; user-select: none; }
.neo-diff-line[data-kind="add"] { color: var(--neo-success); }
.neo-diff-line[data-kind="remove"] { color: var(--neo-danger); }
.neo-diff-line[data-kind="hunk"] { color: var(--neo-muted); background: var(--neo-surface); padding-left: var(--neo-space-m); }

.neo-table-wrap { max-width: 100%; overflow-x: auto; }
.neo-table { border-collapse: collapse; }
.neo-table th, .neo-table td { padding: var(--neo-space-s) var(--neo-space-m); border-bottom: 1px solid var(--neo-border); text-align: left; white-space: nowrap; }
.neo-table th { color: var(--neo-muted); font-weight: var(--neo-title-weight); }
.neo-md-table th, .neo-md-table td { white-space: normal; vertical-align: top; }
.neo-md-table th { color: inherit; }
.neo-table [data-align="center"] { text-align: center; }
.neo-table [data-align="right"] { text-align: right; }

.neo-kv { display: grid; grid-template-columns: max-content 1fr; gap: 0 var(--neo-space-l); margin: 0; }
.neo-kv > dt { color: var(--neo-muted); }
.neo-kv > dd { margin: 0; }
${toneRules((tone) => `.neo-kv > dd[data-tone="${tone}"]`, "color")}

.neo-progress { display: flex; align-items: center; gap: var(--neo-space-m); }
.neo-progress-track { flex: 1; min-width: 6em; height: 8px; border-radius: 4px; background: var(--neo-border); overflow: hidden; }
.neo-progress-bar { width: var(--neo-progress); height: 100%; background: var(--neo-accent); }
.neo-progress-percent { font-variant-numeric: tabular-nums; }
.neo-progress-label { color: var(--neo-muted); }

.neo-button {
  display: inline-flex;
  align-items: center;
  gap: var(--neo-space-s);
  padding: var(--neo-space-s) var(--neo-space-m);
  border: 1px solid var(--neo-border);
  border-radius: 6px;
  color: var(--neo-fg);
  background: var(--neo-surface);
  font: inherit;
  cursor: pointer;
}
.neo-button:hover { border-color: var(--neo-accent); }
.neo-button[data-copied] { border-color: var(--neo-success); color: var(--neo-success); }
.neo-key { padding: 0 0.4em; border: 1px solid var(--neo-border); border-radius: 4px; color: var(--neo-muted); font-family: var(--neo-code-font); font-size: 0.85em; }
`;
