// Theme → CSS custom properties, and the stylesheet that uses only those. The daemon's Theme
// (protocol/themes/default.json) is the single source of colors, spacing and type sizes: nothing
// in the stylesheet below hard-codes them, so a new theme restyles the chat without new CSS.
import type { TextStyle, Theme } from "@suite/protocol/types";

/** Font stacks for a TextStyle's `mono` flag (the theme only says mono or not). */
const FONT_SANS = 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif';
const FONT_MONO = 'ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace';

function textVariables(name: string, style: TextStyle): Record<string, string> {
  let font = "var(--suite-font-sans)";
  if (style.mono) font = "var(--suite-font-mono)";
  return {
    [`--suite-${name}-size`]: `${style.size}px`,
    [`--suite-${name}-weight`]: String(style.weight),
    [`--suite-${name}-font`]: font,
  };
}

/** The custom properties a theme sets on the chat container. */
export function themeVariables(theme: Theme): Record<string, string> {
  const { colors, spacing, text } = theme;
  return {
    "--suite-fg": colors.fg,
    "--suite-bg": colors.bg,
    "--suite-surface": colors.surface,
    "--suite-border": colors.border,
    "--suite-accent": colors.accent,
    "--suite-muted": colors.muted,
    "--suite-success": colors.success,
    "--suite-danger": colors.danger,
    "--suite-space-s": `${spacing.s}px`,
    "--suite-space-m": `${spacing.m}px`,
    "--suite-space-l": `${spacing.l}px`,
    "--suite-font-sans": FONT_SANS,
    "--suite-font-mono": FONT_MONO,
    ...textVariables("title", text.title),
    ...textVariables("body", text.body),
    ...textVariables("code", text.code),
  };
}

/** Selector prefix of every rule: the container mountChat was given. */
export const CHAT_CLASS = "suite-chat";

const TONES = ["accent", "muted", "success", "danger"];

function toneRules(selector: (tone: string) => string, property: string): string {
  return TONES.map((tone) => `${selector(tone)} { ${property}: var(--suite-${tone}); }`).join("\n");
}

export const chatStyles = `
.${CHAT_CLASS} {
  display: flex;
  flex-direction: column;
  gap: var(--suite-space-m);
  box-sizing: border-box;
  padding: var(--suite-space-l);
  overflow-y: auto;
  color: var(--suite-fg);
  background: var(--suite-bg);
  font-family: var(--suite-body-font);
  font-size: var(--suite-body-size);
  font-weight: var(--suite-body-weight);
  line-height: 1.5;
}
.${CHAT_CLASS} *, .${CHAT_CLASS} *::before, .${CHAT_CLASS} *::after { box-sizing: border-box; }
.${CHAT_CLASS} :focus-visible { outline: 2px solid var(--suite-accent); outline-offset: 2px; }

.suite-item { max-width: 100%; }
.suite-item[tabindex] { border-radius: 6px; }
.suite-item-header { color: var(--suite-muted); font-size: 0.9em; margin-bottom: var(--suite-space-s); }
.suite-item[data-kind="user"] {
  align-self: flex-end;
  max-width: 85%;
  padding: var(--suite-space-m) var(--suite-space-l);
  border: 1px solid var(--suite-border);
  border-radius: 12px 12px 2px 12px;
  background: var(--suite-surface);
}
.suite-item[data-kind="tool"] > .suite-item-body { margin-left: var(--suite-space-l); }
.suite-item[data-kind="notice"] { color: var(--suite-muted); font-size: 0.9em; }
.suite-item[data-kind="notice"] .suite-text > p { margin: 0; }
.suite-item[data-kind="thought"] > summary { cursor: pointer; margin-bottom: 0; }
.suite-item[data-kind="thought"][open] > summary { margin-bottom: var(--suite-space-s); }
.suite-item[data-kind="thought"] > .suite-item-body { color: var(--suite-muted); font-style: italic; }

.suite-state { display: inline-block; }
.suite-state[data-state="pending"] { color: var(--suite-muted); }
.suite-state[data-state="running"] { color: var(--suite-accent); animation: suite-spin 1.2s linear infinite; }
.suite-state[data-state="done"] { color: var(--suite-success); }
.suite-state[data-state="failed"] { color: var(--suite-danger); }
@keyframes suite-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .suite-state[data-state="running"] { animation: none; } }

.suite-card {
  padding: var(--suite-space-m) var(--suite-space-l);
  border: 1px solid var(--suite-border);
  border-radius: 8px;
  background: var(--suite-surface);
  display: flex;
  flex-direction: column;
  gap: var(--suite-space-s);
  min-width: 0;
}
.suite-card-title { margin: 0; font-size: var(--suite-title-size); font-weight: var(--suite-title-weight); }
${toneRules((tone) => `.suite-card[data-tone="${tone}"]`, "border-color")}
${toneRules((tone) => `.suite-card[data-tone="${tone}"] > .suite-card-title`, "color")}

.suite-row, .suite-stack { display: flex; gap: var(--suite-space-s); min-width: 0; }
.suite-row { flex-direction: row; flex-wrap: wrap; align-items: flex-start; }
.suite-stack { flex-direction: column; }
.suite-row > *, .suite-stack > * { min-width: 0; }
.suite-row[data-gap="s"], .suite-stack[data-gap="s"] { gap: var(--suite-space-s); }
.suite-row[data-gap="m"], .suite-stack[data-gap="m"] { gap: var(--suite-space-m); }
.suite-row[data-gap="l"], .suite-stack[data-gap="l"] { gap: var(--suite-space-l); }

.suite-text > :first-child { margin-top: 0; }
.suite-text > :last-child { margin-bottom: 0; }
.suite-text p, .suite-text ul, .suite-text ol, .suite-text blockquote, .suite-text pre {
  margin: 0 0 var(--suite-space-s);
}
.suite-text[data-role="title"] { font-size: var(--suite-title-size); font-weight: var(--suite-title-weight); font-family: var(--suite-title-font); }
.suite-text[data-role="code"] { font-size: var(--suite-code-size); font-weight: var(--suite-code-weight); font-family: var(--suite-code-font); }
${toneRules((tone) => `.suite-text[data-tone="${tone}"]`, "color")}
.suite-heading { margin: var(--suite-space-m) 0 var(--suite-space-s); font-size: var(--suite-body-size); font-weight: var(--suite-title-weight); }
h1.suite-heading, h2.suite-heading { font-size: var(--suite-title-size); }
.suite-list { padding-left: calc(var(--suite-space-l) * 1.5); }
.suite-list .suite-list { margin-bottom: 0; }
.suite-list li > p { margin: 0; }
.suite-task { list-style: none; margin-left: calc(var(--suite-space-l) * -1.5); padding-left: 0; }
.suite-checkbox { margin: 0 var(--suite-space-s) 0 0; vertical-align: middle; accent-color: var(--suite-accent); }
.suite-task > p { display: inline; }
.suite-quote { padding-left: var(--suite-space-m); border-left: 3px solid var(--suite-border); color: var(--suite-muted); margin-left: 0; margin-right: 0; }
.suite-rule { border: 0; border-top: 1px solid var(--suite-border); margin: var(--suite-space-m) 0; }
.suite-link { color: var(--suite-accent); text-decoration: underline; }
.suite-inline-code, .suite-code {
  font-family: var(--suite-code-font);
  font-size: var(--suite-code-size);
  font-weight: var(--suite-code-weight);
}
.suite-inline-code { padding: 0 0.3em; border-radius: 4px; background: var(--suite-surface); }
.suite-code {
  margin: 0;
  padding: var(--suite-space-m);
  overflow-x: auto;
  white-space: pre;
  border-radius: 6px;
  background: var(--suite-surface);
}
.suite-card .suite-code, .suite-user .suite-code { background: var(--suite-bg); }
.suite-code > code { font: inherit; }

.suite-image { margin: 0; }
.suite-image img { max-width: 100%; height: auto; border-radius: 6px; }
.suite-image figcaption { color: var(--suite-muted); font-size: 0.9em; }
.suite-image-fallback {
  display: inline-block;
  max-width: 100%;
  padding: var(--suite-space-s) var(--suite-space-m);
  border: 1px dashed var(--suite-border);
  border-radius: 6px;
  color: var(--suite-muted);
  font-family: var(--suite-code-font);
  font-size: var(--suite-code-size);
  overflow-wrap: anywhere;
}

.suite-file {
  display: inline-block;
  padding: 0 var(--suite-space-m);
  border: 1px solid var(--suite-border);
  border-radius: 999px;
  background: var(--suite-surface);
}

.suite-diff { margin: 0; border: 1px solid var(--suite-border); border-radius: 6px; overflow: hidden; }
.suite-diff-path { padding: var(--suite-space-s) var(--suite-space-m); color: var(--suite-muted); background: var(--suite-surface); border-bottom: 1px solid var(--suite-border); font-family: var(--suite-code-font); font-size: var(--suite-code-size); }
.suite-diff-body { margin: 0; overflow-x: auto; font-family: var(--suite-code-font); font-size: var(--suite-code-size); }
.suite-diff-line { display: flex; white-space: pre; padding-right: var(--suite-space-m); }
.suite-diff-num { flex: none; min-width: 3.5ch; padding: 0 var(--suite-space-s); text-align: right; color: var(--suite-muted); user-select: none; }
.suite-diff-sign { flex: none; width: 2ch; text-align: center; user-select: none; }
.suite-diff-line[data-kind="add"] { color: var(--suite-success); }
.suite-diff-line[data-kind="remove"] { color: var(--suite-danger); }
.suite-diff-line[data-kind="hunk"] { color: var(--suite-muted); background: var(--suite-surface); padding-left: var(--suite-space-m); }

.suite-table-wrap { max-width: 100%; overflow-x: auto; }
.suite-table { border-collapse: collapse; }
.suite-table th, .suite-table td { padding: var(--suite-space-s) var(--suite-space-m); border-bottom: 1px solid var(--suite-border); text-align: left; white-space: nowrap; }
.suite-table th { color: var(--suite-muted); font-weight: var(--suite-title-weight); }

.suite-kv { display: grid; grid-template-columns: max-content 1fr; gap: 0 var(--suite-space-l); margin: 0; }
.suite-kv > dt { color: var(--suite-muted); }
.suite-kv > dd { margin: 0; }

.suite-progress { display: flex; align-items: center; gap: var(--suite-space-m); }
.suite-progress-track { flex: 1; min-width: 6em; height: 8px; border-radius: 4px; background: var(--suite-border); overflow: hidden; }
.suite-progress-bar { width: var(--suite-progress); height: 100%; background: var(--suite-accent); }
.suite-progress-percent { font-variant-numeric: tabular-nums; }
.suite-progress-label { color: var(--suite-muted); }

.suite-button {
  display: inline-flex;
  align-items: center;
  gap: var(--suite-space-s);
  padding: var(--suite-space-s) var(--suite-space-m);
  border: 1px solid var(--suite-border);
  border-radius: 6px;
  color: var(--suite-fg);
  background: var(--suite-surface);
  font: inherit;
  cursor: pointer;
}
.suite-button:hover { border-color: var(--suite-accent); }
.suite-button[data-copied] { border-color: var(--suite-success); color: var(--suite-success); }
.suite-key { padding: 0 0.4em; border: 1px solid var(--suite-border); border-radius: 4px; color: var(--suite-muted); font-family: var(--suite-code-font); font-size: 0.85em; }
`;
