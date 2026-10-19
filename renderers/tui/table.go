package tui

import (
	"strings"

	protocol "neoworks.dev/suite/protocol/go"
)

// table draws a box-drawn grid with a header row. Columns that don't fit are
// cut (cells end in …), the widest first, so short columns stay whole.
func (r *renderer) table(v protocol.TableComponent, w int) []line {
	cols := len(v.Columns)
	for _, row := range v.Rows {
		cols = max(cols, len(row))
	}
	if cols == 0 {
		return nil
	}
	cell := func(row []string, c int) string {
		if c < len(row) {
			return oneLine(row[c])
		}
		return ""
	}

	widths := make([]int, cols)
	for c := range widths {
		widths[c] = max(Width(cell(v.Columns, c)), 1)
		for _, row := range v.Rows {
			widths[c] = max(widths[c], Width(cell(row, c)))
		}
	}
	fitWidths(widths, w-(3*cols+1))

	border := borderStyle()
	rule := boxRule(widths)
	rowLine := func(row []string, st style) line {
		l := lit("│", border)
		for c, cw := range widths {
			txt := truncate(cell(row, c), cw)
			l = append(l, run{" " + txt + strings.Repeat(" ", cw-Width(txt)) + " ", st}, run{"│", border})
		}
		return l
	}

	out := []line{rule("┌", "┬", "┐"), rowLine(v.Columns, style{bold: true}), rule("├", "┼", "┤")}
	for _, row := range v.Rows {
		out = append(out, rowLine(row, style{}))
	}
	out = append(out, rule("└", "┴", "┘"))
	return clip(out, w)
}

// clip cuts lines wider than w. It only matters for tables with more columns
// than the width has room for.
func clip(lines []line, w int) []line {
	for i, l := range lines {
		if l.width() <= w {
			continue
		}
		var cut line
		left := w
		for _, r := range l {
			head, _ := splitWidth(r.text, left)
			if Width(head) > left {
				break
			}
			cut = append(cut, run{head, r.st})
			if left -= Width(head); left <= 0 {
				break
			}
		}
		lines[i] = cut
	}
	return lines
}

// fitWidths narrows the widest column a cell at a time until the columns fit
// avail cells together. Each column costs its text plus " │ " (the outer
// edges add 1 more), which the caller has taken off avail.
func fitWidths(widths []int, avail int) {
	total := 0
	for _, cw := range widths {
		total += cw
	}
	for total > avail {
		widest := 0
		for c, cw := range widths {
			if cw > widths[widest] {
				widest = c
			}
		}
		if widths[widest] <= 1 {
			break // can't get any narrower; the clip makes it fit
		}
		widths[widest]--
		total--
	}
}

// boxRule draws a horizontal rule of the box around columns of these widths.
func boxRule(widths []int) func(left, mid, right string) line {
	return func(left, mid, right string) line {
		var b strings.Builder
		b.WriteString(left)
		for c, cw := range widths {
			if c > 0 {
				b.WriteString(mid)
			}
			b.WriteString(strings.Repeat("─", cw+2))
		}
		b.WriteString(right)
		return lit(b.String(), borderStyle())
	}
}

func borderStyle() style { return style{fg: rBorder} }

// tableBlock draws a markdown table in the same box as the table component.
// Cells keep their styles and wrap inside their column instead of being cut;
// once a row takes more than one line, rules separate the rows too.
func (r *renderer) tableBlock(v protocol.TableBlock, w int) []line {
	cols := len(v.Header)
	for _, row := range v.Rows {
		cols = max(cols, len(row))
	}
	if cols == 0 {
		return nil
	}
	cells := func(row []protocol.TableCell, bold bool) [][]run {
		out := make([][]run, cols)
		for c := range out {
			if c < len(row) {
				out[c] = spanRuns(row[c].Spans)
			}
			for i := range out[c] {
				out[c][i].st.bold = out[c][i].st.bold || bold
			}
		}
		return out
	}
	header := cells(v.Header, true)
	body := make([][][]run, len(v.Rows))
	for i, row := range v.Rows {
		body[i] = cells(row, false)
	}

	widths := make([]int, cols)
	for c := range widths {
		widths[c] = 1
		for _, row := range append([][][]run{header}, body...) {
			for _, l := range wrapRuns(row[c], 1<<30) {
				widths[c] = max(widths[c], l.width())
			}
		}
	}
	fitWidths(widths, w-(3*cols+1))

	align := func(c int) protocol.ColumnAlign {
		if c < len(v.Align) {
			return v.Align[c]
		}
		return protocol.ColumnAlignLeft
	}
	rowLines := func(row [][]run) []line {
		wrapped := make([][]line, cols)
		height := 1
		for c, cw := range widths {
			wrapped[c] = wrapRuns(row[c], cw)
			height = max(height, len(wrapped[c]))
		}
		out := make([]line, height)
		for i := range out {
			l := lit("│", borderStyle())
			for c, cw := range widths {
				var cell line
				if i < len(wrapped[c]) {
					cell = wrapped[c][i]
				}
				room := max(cw-cell.width(), 0)
				left := 0
				switch align(c) {
				case protocol.ColumnAlignRight:
					left = room
				case protocol.ColumnAlignCenter:
					left = room / 2
				}
				l = append(l, run{" " + strings.Repeat(" ", left), style{}})
				l = append(l, cell...)
				l = append(l, run{strings.Repeat(" ", room-left) + " ", style{}}, run{"│", borderStyle()})
			}
			out[i] = l
		}
		return out
	}

	rule := boxRule(widths)
	rows := make([][]line, len(body))
	ruled := false
	for i, row := range body {
		rows[i] = rowLines(row)
		ruled = ruled || len(rows[i]) > 1
	}
	out := []line{rule("┌", "┬", "┐")}
	out = append(out, rowLines(header)...)
	out = append(out, rule("├", "┼", "┤"))
	for i, row := range rows {
		if ruled && i > 0 {
			out = append(out, rule("├", "┼", "┤"))
		}
		out = append(out, row...)
	}
	out = append(out, rule("└", "┴", "┘"))
	return clip(out, w)
}
