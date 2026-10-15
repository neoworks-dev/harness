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
	// Each column costs its text plus " │ " (the outer edges add 1 more).
	avail := w - (3*cols + 1)
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
			break // can't get any narrower; the clip below makes it fit
		}
		widths[widest]--
		total--
	}

	border := style{fg: rBorder}
	rule := func(left, mid, right string) line {
		var b strings.Builder
		b.WriteString(left)
		for c, cw := range widths {
			if c > 0 {
				b.WriteString(mid)
			}
			b.WriteString(strings.Repeat("─", cw+2))
		}
		b.WriteString(right)
		return lit(b.String(), border)
	}
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
