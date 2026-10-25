package tui

import (
	"fmt"
	"math"
	"strings"

	protocol "neoworks.dev/neo/protocol/go"
)

// component draws c in w columns.
func (r *renderer) component(c protocol.Component, w int) []line {
	switch v := c.Value.(type) {
	case protocol.CardComponent:
		return r.card(v, w)
	case protocol.RowComponent:
		return r.row(v, w)
	case protocol.StackComponent:
		return r.stack(v.Gap, v.Children, w)
	case protocol.TextComponent:
		return r.textComponent(v, w)
	case protocol.ImageComponent:
		return r.image(v, w)
	case protocol.FileComponent:
		return r.file(v, w)
	case protocol.CodeComponent:
		return r.code(v.Code, w)
	case protocol.DiffComponent:
		return r.diff(v, w)
	case protocol.TableComponent:
		return r.table(v, w)
	case protocol.KvComponent:
		return r.kv(v, w)
	case protocol.ProgressComponent:
		return r.progress(v, w)
	case protocol.ButtonComponent:
		return wrapRuns(r.button(v), w)
	case protocol.LinkComponent:
		return wrapRuns(linkRuns(v.URL, v.Text, style{}), w)
	case protocol.OpenUiComponent:
		return unsupported("openui", w)
	}
	return nil // a component from a newer protocol: nothing to draw
}

// unsupported is the line for a component this renderer knows of and
// doesn't draw (neoui is the terminal app's; docs/neoui.md).
func unsupported(kind string, w int) []line {
	return wrapRuns([]run{{"[unsupported component: " + kind + "]", style{fg: rMuted}}}, w)
}

func (r *renderer) children(cs []protocol.Component, gap int, w int) []line {
	var out []line
	for i, c := range cs {
		ls := r.component(c, w)
		if len(ls) == 0 {
			continue
		}
		if i > 0 && len(out) > 0 {
			for range gap {
				out = append(out, line{})
			}
		}
		out = append(out, ls...)
	}
	return out
}

// gapLines is the TUI's reading of the theme's s/m/l spacing: 0, 0 and 1
// blank lines (renderers/README.md).
func gapLines(g *protocol.Space) int {
	if g != nil && *g == protocol.SpaceL {
		return 1
	}
	return 0
}

func (r *renderer) stack(gap *protocol.Space, cs []protocol.Component, w int) []line {
	return r.children(cs, gapLines(gap), w)
}

// row puts buttons side by side and stacks everything else: terminals have
// no layout to wrap a row of cards into.
func (r *renderer) row(v protocol.RowComponent, w int) []line {
	allButtons := len(v.Children) > 0
	for _, c := range v.Children {
		if _, ok := c.Value.(protocol.ButtonComponent); !ok {
			allButtons = false
		}
	}
	if !allButtons {
		return r.stack(v.Gap, v.Children, w)
	}
	var row line
	for i, c := range v.Children {
		if i > 0 {
			row = append(row, run{"  ", style{}})
		}
		row = append(row, r.button(c.Value.(protocol.ButtonComponent))...)
	}
	if row.width() <= w {
		return []line{row}
	}
	// Doesn't fit: stack them. The buttons were already numbered above, so
	// rewind and draw again.
	r.next -= len(v.Children)
	return r.children(v.Children, 0, w)
}

// button draws "[k] Label"; k is the key Buttons reports for it.
func (r *renderer) button(b protocol.ButtonComponent) line {
	key := "?"
	if r.next < len(r.btns) {
		key = r.btns[r.next].Key
	}
	r.next++
	return line{
		{"[" + clean(key) + "]", style{fg: rAccent, bold: true}},
		{" " + clean(b.Label), style{}},
	}
}

func (r *renderer) card(v protocol.CardComponent, w int) []line {
	title := ""
	if v.Title != nil {
		title = strings.ReplaceAll(clean(*v.Title), "\n", " ")
	}
	tone := toneRole(v.Tone)
	titleStyle := style{fg: tone, bold: true}

	if w < 20 { // no room for a frame: a title line and indentation
		var out []line
		if title != "" {
			out = append(out, wrapRuns([]run{{title, titleStyle}}, w)...)
		}
		return append(out, indented(r.children(v.Children, 0, w-2), 2)...)
	}

	border := style{fg: rBorder}
	if tone != rNone {
		border.fg = tone
	}
	inner := w - 4
	var top line
	if title == "" {
		top = lit("╭"+strings.Repeat("─", w-2)+"╮", border)
	} else {
		title = truncate(title, w-6)
		fill := w - Width(title) - 5
		top = join(lit("╭─ ", border), lit(title, titleStyle), lit(" "+strings.Repeat("─", fill)+"╮", border))
	}
	out := []line{top}
	for _, l := range r.children(v.Children, 0, inner) {
		out = append(out, join(lit("│ ", border), l.padTo(inner, style{}), lit(" │", border)))
	}
	return append(out, lit("╰"+strings.Repeat("─", w-2)+"╯", border))
}

func (r *renderer) image(v protocol.ImageComponent, w int) []line {
	label := "[image "
	if v.Width != nil && v.Height != nil {
		label += fmt.Sprintf("%d×%d · ", *v.Width, *v.Height)
	}
	label += clean(v.Src) + "]"
	out := wrapRuns([]run{{oneLine(label), style{}}}, w)
	if v.Caption != nil && *v.Caption != "" {
		out = append(out, wrapRuns([]run{{clean(*v.Caption), style{fg: rMuted}}}, w)...)
	}
	return out
}

func (r *renderer) file(v protocol.FileComponent, w int) []line {
	p := strings.ReplaceAll(clean(v.Path), "\n", " ")
	s := "📄 " + p
	if v.Name != nil && *v.Name != "" && *v.Name != p {
		s = "📄 " + strings.ReplaceAll(clean(*v.Name), "\n", " ") + " (" + p + ")"
	}
	return wrapRuns([]run{{oneLine(s), style{}}}, w)
}

// code draws a monospace block: indented, hard-wrapped, on the surface color.
func (r *renderer) code(code string, w int) []line {
	st := style{fg: rFg, bg: rSurface}
	if w < 4 {
		return hardWrap(code, w, style{})
	}
	var out []line
	for _, l := range hardWrap(code, w-2, st) {
		out = append(out, join(lit("  ", st), l).padTo(w, st))
	}
	return out
}

func (r *renderer) diff(v protocol.DiffComponent, w int) []line {
	var out []line
	if v.Path != nil && *v.Path != "" {
		out = append(out, hardWrap(strings.ReplaceAll(*v.Path, "\n", " "), w, style{bold: true})...)
	}
	for _, dl := range v.Lines {
		mark, st := " ", style{}
		switch dl.Kind {
		case protocol.DiffLineKindAdd:
			mark, st = "+", style{fg: rSuccess}
		case protocol.DiffLineKindRemove:
			mark, st = "-", style{fg: rDanger}
		case protocol.DiffLineKindHunk:
			mark, st = "", style{fg: rMuted}
		}
		body := hardWrap(strings.ReplaceAll(dl.Text, "\n", " "), w-len(mark), st)
		// Continuation lines of a wrapped diff line keep the column of the mark.
		out = append(out, prefixed(body, lit(mark, st), lit(strings.Repeat(" ", len(mark)), st))...)
	}
	return out
}

func (r *renderer) kv(v protocol.KvComponent, w int) []line {
	keyW := 0
	for _, it := range v.Items {
		keyW = max(keyW, Width(oneLine(it.Key)))
	}
	keyW = min(keyW, max(w/2, 1))
	var out []line
	for _, it := range v.Items {
		key := truncate(oneLine(it.Key), keyW)
		head := join(lit(key, style{fg: rMuted}), lit(strings.Repeat(" ", keyW-Width(key)+2), style{}))
		body := wrapRuns([]run{{clean(it.Value), style{}}}, max(w-keyW-2, 1))
		if len(body) == 0 {
			body = []line{{}}
		}
		out = append(out, prefixed(body, head, lit(strings.Repeat(" ", keyW+2), style{}))...)
	}
	return out
}

// oneLine flattens text that must stay on a single line (keys, table cells).
func oneLine(s string) string {
	return strings.Join(strings.Fields(strings.ReplaceAll(clean(s), "\n", " ")), " ")
}

const barCells = 20

// progress draws "▓▓▓░░ 60% label" with a 20-cell bar.
func (r *renderer) progress(v protocol.ProgressComponent, w int) []line {
	val := math.Min(math.Max(v.Value, 0), 1)
	if math.IsNaN(v.Value) {
		val = 0
	}
	percent := fmt.Sprintf(" %d%%", int(math.Round(val*100)))
	cells := min(barCells, max(w-len(percent), 1))
	filled := int(math.Round(val * float64(cells)))
	bar := join(
		lit(strings.Repeat("▓", filled), style{fg: rAccent}),
		lit(strings.Repeat("░", cells-filled), style{fg: rMuted}),
		lit(percent, style{}),
	)
	if v.Label == nil || *v.Label == "" {
		return []line{bar}
	}
	label := oneLine(*v.Label)
	if bar.width()+1+Width(label) <= w {
		return []line{join(bar, lit(" "+label, style{fg: rMuted}))}
	}
	return append([]line{bar}, wrapRuns([]run{{label, style{fg: rMuted}}}, w)...)
}

// linkRuns draws a link as underlined text followed by its URL in parentheses,
// or just the URL when there is no distinct text.
func linkRuns(url string, label *string, base style) []run {
	url = oneLine(url)
	st := base
	st.underline = true
	if label == nil || oneLine(*label) == "" || oneLine(*label) == url {
		return []run{{url, st}}
	}
	return []run{{oneLine(*label), st}, {" (" + url + ")", style{fg: rMuted}}}
}
