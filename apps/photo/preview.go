package main

import (
	"fmt"
	"image"
	"strconv"
	"strings"

	"neoworks.dev/suite/renderers/tui"
)

// previewBackground is what transparent pixels are shown over.
var previewBackground = [3]int{0x20, 0x20, 0x20}

// previewSize is the size in pixels an image of iw×ih is drawn at inside a
// box of cols×rows terminal cells. A cell shows two pixels, one above the
// other (a half block), so the box is cols×2*rows pixels. Images are shrunk to
// fit and never enlarged; the height is rounded up to a whole cell.
func previewSize(iw, ih, cols, rows int) (w, h int) {
	boxW, boxH := cols, rows*2
	if iw < 1 || ih < 1 || boxW < 1 || boxH < 1 {
		return 0, 0
	}
	w, h = iw, ih
	if w > boxW || h > boxH {
		// The smaller of the two scale factors, in integer arithmetic.
		if w*boxH > h*boxW {
			w, h = boxW, max(1, ih*boxW/iw)
		} else {
			w, h = max(1, iw*boxH/ih), boxH
		}
	}
	return w, h
}

// renderPreview draws img with half blocks into at most cols×rows cells, one
// string per row without a trailing newline. Each cell is "▀" with the
// upper pixel as its foreground and the lower as its background. TrueColor
// uses 24-bit colors, ANSI256 the xterm palette; with fewer colors it falls
// back to a gray ramp of plain characters, so it also reads in a pipe.
func renderPreview(img *image.NRGBA, cols, rows int, mode tui.ColorMode) []string {
	w, h := previewSize(img.Bounds().Dx(), img.Bounds().Dy(), cols, rows)
	if w == 0 {
		return nil
	}
	small := img
	if w != img.Bounds().Dx() || h != img.Bounds().Dy() {
		small = areaResize(img, w, h)
	}
	pixel := func(x, y int) [3]int {
		if y >= h {
			return previewBackground // the odd last row: nothing below it
		}
		p := small.Pix[small.PixOffset(x, y):][:4]
		a := int(p[3])
		var c [3]int
		for i := range c {
			c[i] = (int(p[i])*a + previewBackground[i]*(255-a) + 127) / 255
		}
		return c
	}

	lines := make([]string, 0, (h+1)/2)
	for y := 0; y < h; y += 2 {
		var b strings.Builder
		var lastFg, lastBg string
		for x := range w {
			top, bottom := pixel(x, y), pixel(x, y+1)
			switch mode {
			case tui.TrueColor, tui.ANSI256:
				fg, bg := colorCode(38, top, mode), colorCode(48, bottom, mode)
				if fg != lastFg {
					b.WriteString(fg)
					lastFg = fg
				}
				if bg != lastBg {
					b.WriteString(bg)
					lastBg = bg
				}
				b.WriteString("▀")
			default:
				b.WriteByte(rampChar((luma(top) + luma(bottom)) / 2))
			}
		}
		if mode == tui.TrueColor || mode == tui.ANSI256 {
			b.WriteString("\x1b[0m")
		}
		lines = append(lines, b.String())
	}
	return lines
}

// colorCode is the escape that sets the foreground (38) or background (48).
func colorCode(base int, c [3]int, mode tui.ColorMode) string {
	if mode == tui.TrueColor {
		return fmt.Sprintf("\x1b[%d;2;%d;%d;%dm", base, c[0], c[1], c[2])
	}
	return "\x1b[" + strconv.Itoa(base) + ";5;" + strconv.Itoa(ansi256(c)) + "m"
}

// ansi256 is the xterm palette index closest to c: the 6×6×6 color cube
// (16–231) or the gray ramp (232–255), whichever is nearer.
func ansi256(c [3]int) int {
	level := func(v int) int { // nearest of the cube levels 0, 95, 135, 175, 215, 255
		switch {
		case v < 48:
			return 0
		case v < 115:
			return 1
		}
		return (v - 35) / 40
	}
	cubeValue := func(l int) int {
		if l == 0 {
			return 0
		}
		return 55 + 40*l
	}
	r, g, b := level(c[0]), level(c[1]), level(c[2])
	cube := 16 + 36*r + 6*g + b
	cr, cg, cb := cubeValue(r), cubeValue(g), cubeValue(b)

	avg := (c[0] + c[1] + c[2]) / 3
	gray := min(max((avg-8+5)/10, 0), 23)
	gv := 8 + 10*gray

	dist := func(a, b, c2, x, y, z int) int { return (a-x)*(a-x) + (b-y)*(b-y) + (c2-z)*(c2-z) }
	if dist(c[0], c[1], c[2], gv, gv, gv) < dist(c[0], c[1], c[2], cr, cg, cb) {
		return 232 + gray
	}
	return cube
}

func luma(c [3]int) int { return (299*c[0] + 587*c[1] + 114*c[2]) / 1000 }

const ramp = " .:-=+*#%@"

// rampChar picks a character darker or lighter by brightness 0..255.
func rampChar(l int) byte { return ramp[min(max(l, 0), 255)*(len(ramp)-1)/255] }
