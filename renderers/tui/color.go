package tui

import (
	"fmt"
	"strconv"
	"strings"

	protocol "neoworks.dev/neo/protocol/go"
)

// ColorMode is how much color the terminal can show.
type ColorMode int

const (
	// None emits no escape codes at all: plain text.
	None ColorMode = iota
	// ANSI16 uses the 16 standard colors.
	ANSI16
	// ANSI256 uses the xterm 256-color palette.
	ANSI256
	// TrueColor emits 24-bit colors.
	TrueColor
)

func (m ColorMode) String() string {
	return [...]string{"none", "ansi16", "ansi256", "truecolor"}[m]
}

// role is a theme color by meaning. Text without a role keeps the terminal's
// own default colors, so output stays readable on light and dark terminals.
type role uint8

const (
	rNone role = iota
	rFg
	rMuted
	rAccent
	rSuccess
	rDanger
	rBorder
	rSurface
	numRoles
)

func toneRole(t *protocol.Tone) role {
	if t == nil {
		return rNone
	}
	switch *t {
	case protocol.ToneAccent:
		return rAccent
	case protocol.ToneMuted:
		return rMuted
	case protocol.ToneSuccess:
		return rSuccess
	case protocol.ToneDanger:
		return rDanger
	}
	return rNone
}

type rgb struct{ r, g, b uint8 }

// parseHex reads #rrggbb (or #rgb).
func parseHex(s string) (rgb, bool) {
	s = strings.TrimPrefix(s, "#")
	if len(s) == 3 {
		s = string([]byte{s[0], s[0], s[1], s[1], s[2], s[2]})
	}
	if len(s) != 6 {
		return rgb{}, false
	}
	v, err := strconv.ParseUint(s, 16, 32)
	if err != nil {
		return rgb{}, false
	}
	return rgb{uint8(v >> 16), uint8(v >> 8), uint8(v)}, true
}

func (c rgb) dist(o rgb) int {
	dr, dg, db := int(c.r)-int(o.r), int(c.g)-int(o.g), int(c.b)-int(o.b)
	return dr*dr + dg*dg + db*db
}

// grays are the 16-color palette's grays by index, with their lightness.
var grays = [...]struct{ index, level int }{{0, 0}, {8, 127}, {7, 229}, {15, 255}}

// nearest16 maps c to one of the 16 standard colors other than skip. Plain
// RGB distance would turn a pastel red into gray, because the palette has no
// pastels, so this goes by meaning instead: low-chroma colors become the
// nearest gray, others the nearest hue, in its bright variant when light (so pastel red is bright red, not gray).
func nearest16(c rgb, skip int) int {
	hi, lo := max(c.r, c.g, c.b), min(c.r, c.g, c.b)
	chroma := int(hi) - int(lo)
	if chroma < 40 {
		level := (int(hi) + int(lo)) / 2
		best, bestD := -1, 0
		for _, g := range grays {
			d := (level - g.level) * (level - g.level)
			if g.index != skip && (best < 0 || d < bestD) {
				best, bestD = g.index, d
			}
		}
		return best
	}
	// A channel is on when it is well above the lowest one; the on channels
	// pick the hue. The 60% bar keeps a yellowish green green.
	on := func(v uint8) bool { return (int(v)-int(lo))*10 >= chroma*6 }
	idx := 0
	switch r, g, b := on(c.r), on(c.g), on(c.b); {
	case r && g:
		idx = 3 // yellow
	case g && b:
		idx = 6 // cyan
	case r && b:
		idx = 5 // magenta
	case r:
		idx = 1
	case g:
		idx = 2
	default:
		idx = 4
	}
	if hi >= 230 { // halfway between the normal (205) and bright (255) intensity
		idx += 8
	}
	if idx == skip {
		idx ^= 8
	}
	return idx
}

// cubeLevels are the channel values of the 6x6x6 color cube (indexes 16-231).
var cubeLevels = [6]int{0, 95, 135, 175, 215, 255}

// ansi256 returns the rgb value of palette index i (16 and up).
func ansi256(i int) rgb {
	if i >= 232 {
		v := uint8(8 + 10*(i-232))
		return rgb{v, v, v}
	}
	i -= 16
	return rgb{uint8(cubeLevels[i/36]), uint8(cubeLevels[i/6%6]), uint8(cubeLevels[i%6])}
}

// nearest256 returns the closest palette index other than skip among the cube
// and the gray ramp. The first 16 are left out: their look depends on the terminal theme.
func nearest256(c rgb, skip int) int {
	best, bestD := -1, 0
	for i := 16; i < 256; i++ {
		if i == skip {
			continue
		}
		if d := c.dist(ansi256(i)); best < 0 || d < bestD {
			best, bestD = i, d
		}
	}
	return best
}

// palette holds the SGR parameters for each role in one color mode.
type palette struct {
	mode   ColorMode
	fg, bg [numRoles]string
}

// newPalette maps the theme's colors to what the terminal can show. A
// foreground that would land on the same color as the theme's background
// (a dark border on a dark theme maps to black in 16 colors) takes the
// next nearest instead, so it never vanishes.
func newPalette(theme protocol.Theme, mode ColorMode) *palette {
	p := &palette{mode: mode}
	if mode == None {
		return p
	}
	c := theme.Colors
	hex := [numRoles]string{
		rFg: c.Fg, rMuted: c.Muted, rAccent: c.Accent, rSuccess: c.Success,
		rDanger: c.Danger, rBorder: c.Border, rSurface: c.Surface,
	}
	skip := -1 // palette index of the theme background
	if bg, ok := parseHex(c.Bg); ok {
		skip = p.index(bg)
	}
	for r, h := range hex {
		col, ok := parseHex(h)
		switch {
		case !ok:
		case role(r) == rSurface:
			p.bg[r] = p.code(col, true, -1)
		default:
			p.fg[r] = p.code(col, false, skip)
		}
	}
	return p
}

// index returns the palette index for the 16 and 256 modes (-1 for true color).
func (p *palette) index(c rgb) int {
	switch p.mode {
	case ANSI16:
		return nearest16(c, -1)
	case ANSI256:
		return nearest256(c, -1)
	}
	return -1
}

// code returns the SGR parameters selecting c as foreground or background.
// A result equal to index skip is replaced by the next nearest color.
func (p *palette) code(c rgb, bg bool, skip int) string {
	switch p.mode {
	case TrueColor:
		base := 38
		if bg {
			base = 48
		}
		return fmt.Sprintf("%d;2;%d;%d;%d", base, c.r, c.g, c.b)
	case ANSI256:
		i := nearest256(c, -1)
		if i == skip {
			i = nearest256(c, skip)
		}
		base := 38
		if bg {
			base = 48
		}
		return fmt.Sprintf("%d;5;%d", base, i)
	case ANSI16:
		i := nearest16(c, -1)
		if i == skip {
			i = nearest16(c, skip)
		}
		base := 30
		if bg {
			base = 40
		}
		if i >= 8 {
			base += 60
			i -= 8
		}
		return strconv.Itoa(base + i)
	}
	return ""
}
