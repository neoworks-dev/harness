package tui

import "strings"

// token is a word, a run of spaces or a forced line break.
type token struct {
	runs  []run
	w     int
	space bool
	nl    bool
}

// tokenize splits styled text at spaces and newlines. A word may span runs
// of different styles ("(" + code + ")"), which stay together when wrapping.
func tokenize(runs []run) []token {
	var toks []token
	var word *token
	endWord := func() {
		if word != nil {
			toks = append(toks, *word)
			word = nil
		}
	}
	for _, r := range runs {
		for _, ch := range r.text {
			switch ch {
			case '\n':
				endWord()
				toks = append(toks, token{nl: true})
			case ' ', '\t':
				endWord()
				if n := len(toks); n > 0 && toks[n-1].space && toks[n-1].runs[0].st == r.st {
					toks[n-1].runs[0].text += " "
					toks[n-1].w++
				} else {
					toks = append(toks, token{runs: []run{{" ", r.st}}, w: 1, space: true})
				}
			default:
				if word == nil {
					word = &token{}
				}
				word.w += runeWidth(ch)
				if n := len(word.runs); n > 0 && word.runs[n-1].st == r.st {
					word.runs[n-1].text += string(ch)
				} else {
					word.runs = append(word.runs, run{string(ch), r.st})
				}
			}
		}
	}
	endWord()
	return toks
}

// wrapRuns breaks styled text into lines of at most w cells. Words wrap at
// spaces; a word longer than a line is cut. Newlines in the text are kept.
func wrapRuns(runs []run, w int) []line {
	var lines []line
	var cur line
	curW := 0
	var gap *token // spaces seen since the last word
	newline := func() {
		lines = append(lines, cur)
		cur, curW, gap = nil, 0, nil
	}
	for _, t := range tokenize(runs) {
		switch {
		case t.nl:
			newline()
		case t.space:
			if curW > 0 {
				g := t
				gap = &g
			}
		default:
			sp := 0
			if gap != nil {
				sp = gap.w
			}
			if curW > 0 && curW+sp+t.w > w {
				newline()
				sp = 0
			}
			if curW > 0 {
				if gap != nil {
					cur = append(cur, gap.runs...)
				}
				cur = append(cur, t.runs...)
				curW += sp + t.w
				gap = nil
				continue
			}
			// Start of a line: cut words that are longer than the line.
			rest := t.runs
			for restW := t.w; restW > w; {
				var head line
				head, rest = cutRuns(rest, w)
				lines = append(lines, head)
				restW = line(rest).width()
			}
			cur, curW, gap = append(line(nil), rest...), line(rest).width(), nil
		}
	}
	if curW > 0 {
		lines = append(lines, cur)
	}
	return lines
}

// cutRuns splits runs after w cells. It always takes at least one rune.
func cutRuns(runs []run, w int) (head line, rest []run) {
	left := w
	for i, r := range runs {
		h, t := splitWidth(r.text, left)
		if len(head) > 0 && Width(h) > left { // a wide rune that doesn't fit
			return head, runs[i:]
		}
		head = append(head, run{h, r.st})
		left -= Width(h)
		if t != "" {
			return head, append([]run{{t, r.st}}, runs[i+1:]...)
		}
		if left <= 0 {
			return head, runs[i+1:]
		}
	}
	return head, nil
}

// hardWrap breaks each line of s after w cells, with no regard for words:
// code keeps its spacing. Every returned line is at most w cells wide.
func hardWrap(s string, w int, st style) []line {
	var lines []line
	for _, raw := range strings.Split(strings.TrimRight(clean(s), "\n"), "\n") {
		for {
			head, tail := splitWidth(raw, w)
			lines = append(lines, lit(head, st))
			if tail == "" {
				break
			}
			raw = tail
		}
	}
	return lines
}
