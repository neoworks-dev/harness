package tui

import (
	"strconv"
	"strings"

	protocol "neoworks.dev/suite/protocol/go"
)

// textComponent draws the blocks of a text component. The role picks a style
// for all of it and the tone a color for whatever has none of its own.
func (r *renderer) textComponent(v protocol.TextComponent, w int) []line {
	lines := r.blocks(v.Blocks, w, true)
	if v.Role != nil {
		switch *v.Role {
		case protocol.TextRoleTitle:
			lines = restyle(lines, func(s style) style { s.bold = true; return s })
		case protocol.TextRoleCode:
			lines = restyle(lines, codeStyle)
		}
	}
	return tint(lines, toneRole(v.Tone))
}

func codeStyle(s style) style {
	s.fg, s.bg = rFg, rSurface
	return s
}

// blocks draws blocks in order. Loose blocks are separated by a blank line;
// tight ones (inside list items) stack directly.
func (r *renderer) blocks(bs []protocol.Block, w int, loose bool) []line {
	var out []line
	for _, b := range bs {
		ls := r.block(b, w)
		if len(ls) == 0 {
			continue
		}
		if loose && len(out) > 0 {
			out = append(out, line{})
		}
		out = append(out, ls...)
	}
	return out
}

func (r *renderer) block(b protocol.Block, w int) []line {
	switch v := b.Value.(type) {
	case protocol.ParagraphBlock:
		return wrapRuns(spanRuns(v.Spans), w)
	case protocol.HeadingBlock:
		// Level 1 and 2 are the title role, which is bold; so is every
		// other level, and the terminal has no sizes to tell them apart.
		runs := spanRuns(v.Spans)
		for i := range runs {
			runs[i].st.bold = true
		}
		return wrapRuns(runs, w)
	case protocol.ListBlock:
		return r.list(v, w)
	case protocol.CodeBlock:
		return r.code(v.Code, w)
	case protocol.QuoteBlock:
		bar := lit("│ ", style{fg: rMuted})
		inner := tint(r.blocks(v.Blocks, max(w-2, 1), true), rMuted)
		out := prefixed(inner, bar, bar)
		for i, l := range inner {
			if len(l) == 0 {
				out[i] = lit("│", style{fg: rMuted}) // keep the bar through blank lines
			}
		}
		return out
	case protocol.RuleBlock:
		return []line{lit(strings.Repeat("─", w), style{fg: rBorder})}
	case protocol.TableBlock:
		return r.tableBlock(v, w)
	case protocol.OpenUiBlock:
		return unsupported("openui", w)
	}
	return nil
}

// spanRuns turns spans into styled runs. A link is its text underlined,
// followed by the URL unless that is the text.
func spanRuns(spans []protocol.Span) []run {
	var out []run
	for _, s := range spans {
		st := style{
			bold:   flag(s.Bold),
			italic: flag(s.Italic),
			strike: flag(s.Strike),
		}
		if flag(s.Code) {
			st = codeStyle(st)
		}
		txt := clean(s.Text)
		if s.Href != nil && *s.Href != "" {
			out = append(out, linkRuns(*s.Href, &txt, st)...)
			continue
		}
		out = append(out, run{txt, st})
	}
	return out
}

func flag(b *bool) bool { return b != nil && *b }

// list draws bullets (•), numbers (1.) and task boxes ([x]/[ ]). Item text
// hangs under the marker's text, and nested lists sit under the item's text.
func (r *renderer) list(v protocol.ListBlock, w int) []line {
	start := int64(1)
	if v.Start != nil {
		start = *v.Start
	}
	numW := 0
	if v.Ordered {
		numW = len(strconv.FormatInt(start+int64(len(v.Items))-1, 10))
	}
	var out []line
	for i, it := range v.Items {
		marker := "• "
		if v.Ordered {
			marker = strconv.FormatInt(start+int64(i), 10)
			marker = strings.Repeat(" ", numW-len(marker)) + marker + ". "
		}
		if it.Checked != nil {
			box := "[ ] "
			if *it.Checked {
				box = "[x] "
			}
			if v.Ordered {
				marker += box
			} else {
				marker = box
			}
		}
		pad := Width(marker)
		body := r.blocks(it.Blocks, max(w-pad, 1), false)
		if len(body) == 0 {
			body = []line{{}}
		}
		out = append(out, prefixed(body, lit(marker, style{}), lit(strings.Repeat(" ", pad), style{}))...)
	}
	return out
}
