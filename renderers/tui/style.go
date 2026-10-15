package tui

import (
	"slices"
	"strings"
)

// style is a run's look: theme roles for colors, plus text attributes.
type style struct {
	fg, bg                               role
	bold, dim, italic, underline, strike bool
}

// run is text with one style. Layout works on lines of runs, so widths never
// have to be measured through escape codes; escapes are only produced at the
// very end, by serialize.
type run struct {
	text string
	st   style
}

type line []run

func (l line) width() int {
	n := 0
	for _, r := range l {
		n += Width(r.text)
	}
	return n
}

// lit is a line of one run.
func lit(s string, st style) line {
	if s == "" {
		return line{}
	}
	return line{{s, st}}
}

// join concatenates runs into one line.
func join(parts ...line) line {
	var out line
	for _, p := range parts {
		out = append(out, p...)
	}
	return out
}

// padTo appends spaces so the line is w cells wide. A background style makes
// the padding visible, as in code blocks.
func (l line) padTo(w int, st style) line {
	if n := w - l.width(); n > 0 {
		return append(slices.Clone(l), run{strings.Repeat(" ", n), st})
	}
	return l
}

// prefixed puts first before the first line and rest before every other.
func prefixed(lines []line, first, rest line) []line {
	out := make([]line, len(lines))
	for i, l := range lines {
		p := rest
		if i == 0 {
			p = first
		}
		out[i] = join(p, l)
	}
	return out
}

func indented(lines []line, n int) []line {
	pad := lit(strings.Repeat(" ", n), style{})
	return prefixed(lines, pad, pad)
}

// restyle applies f to the style of every run.
func restyle(lines []line, f func(style) style) []line {
	out := make([]line, len(lines))
	for i, l := range lines {
		out[i] = make(line, len(l))
		for j, r := range l {
			out[i][j] = run{r.text, f(r.st)}
		}
	}
	return out
}

// tint gives the foreground color fg to runs that have none yet.
func tint(lines []line, fg role) []line {
	if fg == rNone {
		return lines
	}
	return restyle(lines, func(s style) style {
		if s.fg == rNone {
			s.fg = fg
		}
		return s
	})
}

// serialize turns lines into text with the escape codes of pal's mode.
func serialize(lines []line, pal *palette) string {
	var b strings.Builder
	for i, l := range lines {
		if i > 0 {
			b.WriteByte('\n')
		}
		writeLine(&b, l, pal)
	}
	return b.String()
}

func writeLine(b *strings.Builder, l line, pal *palette) {
	// Trailing blanks are noise, unless they carry a background.
	for len(l) > 0 {
		last := l[len(l)-1]
		if strings.Trim(last.text, " ") != "" || (last.st.bg != rNone && pal.mode != None) {
			break
		}
		l = l[:len(l)-1]
	}
	if pal.mode == None {
		var plain strings.Builder
		for _, r := range l {
			plain.WriteString(r.text)
		}
		b.WriteString(strings.TrimRight(plain.String(), " "))
		return
	}
	for i := 0; i < len(l); {
		// Merge neighbors of the same style into one escape sequence.
		st, j := l[i].st, i
		var text strings.Builder
		for ; j < len(l) && l[j].st == st; j++ {
			text.WriteString(l[j].text)
		}
		i = j
		codes := pal.sgr(st)
		if codes == "" {
			b.WriteString(text.String())
			continue
		}
		b.WriteString("\x1b[" + codes + "m" + text.String() + "\x1b[0m")
	}
}

// sgr returns the semicolon-separated SGR parameters for st ("" for plain).
func (p *palette) sgr(st style) string {
	var codes []string
	add := func(on bool, code string) {
		if on {
			codes = append(codes, code)
		}
	}
	add(st.bold, "1")
	add(st.dim, "2")
	add(st.italic, "3")
	add(st.underline, "4")
	add(st.strike, "9")
	if c := p.fg[st.fg]; c != "" {
		codes = append(codes, c)
	}
	if c := p.bg[st.bg]; c != "" {
		codes = append(codes, c)
	}
	return strings.Join(codes, ";")
}
