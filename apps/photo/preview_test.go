package main

import (
	"image"
	"image/color"
	"strings"
	"testing"

	"neoworks.dev/suite/renderers/tui"
)

// quad is a 2×2 image: red and green over blue and white.
func quad() *image.NRGBA {
	img := image.NewNRGBA(image.Rect(0, 0, 2, 2))
	img.SetNRGBA(0, 0, color.NRGBA{255, 0, 0, 255})
	img.SetNRGBA(1, 0, color.NRGBA{0, 255, 0, 255})
	img.SetNRGBA(0, 1, color.NRGBA{0, 0, 255, 255})
	img.SetNRGBA(1, 1, color.NRGBA{255, 255, 255, 255})
	return img
}

func TestPreviewTrueColor(t *testing.T) {
	got := renderPreview(quad(), 10, 5, tui.TrueColor)
	// One cell per column, two pixel rows per cell row: the upper pixel is
	// the foreground of the half block, the lower one its background.
	want := "\x1b[38;2;255;0;0m\x1b[48;2;0;0;255m▀" +
		"\x1b[38;2;0;255;0m\x1b[48;2;255;255;255m▀" +
		"\x1b[0m"
	if len(got) != 1 || got[0] != want {
		t.Errorf("got %q\nwant %q", got, want)
	}
}

func TestPreviewANSI256(t *testing.T) {
	got := renderPreview(quad(), 10, 5, tui.ANSI256)
	// red 196, blue 21, green 46, white 231 in the xterm palette.
	want := "\x1b[38;5;196m\x1b[48;5;21m▀\x1b[38;5;46m\x1b[48;5;231m▀\x1b[0m"
	if len(got) != 1 || got[0] != want {
		t.Errorf("got %q\nwant %q", got, want)
	}
}

func TestPreviewRepeatsNoColorCodes(t *testing.T) {
	img := image.NewNRGBA(image.Rect(0, 0, 3, 2))
	for x := range 3 {
		img.SetNRGBA(x, 0, color.NRGBA{10, 20, 30, 255})
		img.SetNRGBA(x, 1, color.NRGBA{40, 50, 60, 255})
	}
	got := renderPreview(img, 10, 5, tui.TrueColor)
	want := "\x1b[38;2;10;20;30m\x1b[48;2;40;50;60m▀▀▀\x1b[0m"
	if len(got) != 1 || got[0] != want {
		t.Errorf("got %q", got)
	}
}

func TestPreviewWithoutColor(t *testing.T) {
	img := image.NewNRGBA(image.Rect(0, 0, 3, 2))
	for x := range 3 {
		v := uint8(x * 127)
		img.SetNRGBA(x, 0, color.NRGBA{v, v, v, 255})
		img.SetNRGBA(x, 1, color.NRGBA{v, v, v, 255})
	}
	got := renderPreview(img, 10, 5, tui.None)
	if len(got) != 1 || got[0] != " =%" {
		t.Errorf("got %q", got)
	}
	if strings.Contains(got[0], "\x1b") {
		t.Error("escape codes without color")
	}
}

func TestPreviewTransparencyShowsBackground(t *testing.T) {
	img := image.NewNRGBA(image.Rect(0, 0, 1, 2)) // fully transparent
	got := renderPreview(img, 4, 2, tui.TrueColor)
	if len(got) != 1 || !strings.Contains(got[0], "38;2;32;32;32m") || !strings.Contains(got[0], "48;2;32;32;32m") {
		t.Errorf("got %q", got)
	}
}

func TestPreviewOddHeight(t *testing.T) {
	img := image.NewNRGBA(image.Rect(0, 0, 1, 3))
	for y := range 3 {
		img.SetNRGBA(0, y, color.NRGBA{200, 0, 0, 255})
	}
	got := renderPreview(img, 4, 4, tui.TrueColor)
	if len(got) != 2 {
		t.Fatalf("got %d rows: %q", len(got), got)
	}
	// The last row has a pixel on top and the background below.
	if !strings.Contains(got[1], "38;2;200;0;0m\x1b[48;2;32;32;32m▀") {
		t.Errorf("last row %q", got[1])
	}
}

func TestPreviewSize(t *testing.T) {
	for _, tc := range []struct{ iw, ih, cols, rows, w, h int }{
		{2, 2, 10, 5, 2, 2},        // small images are not enlarged
		{100, 50, 40, 20, 40, 20},  // limited by width: 40 cols, aspect 2:1
		{100, 100, 40, 10, 20, 20}, // limited by height: 10 rows = 20 pixels
		{1000, 1, 40, 10, 40, 1},   // never below one pixel
		{0, 5, 10, 10, 0, 0},
		{5, 5, 0, 10, 0, 0},
	} {
		w, h := previewSize(tc.iw, tc.ih, tc.cols, tc.rows)
		if w != tc.w || h != tc.h {
			t.Errorf("previewSize(%d×%d in %d×%d) = %d×%d, want %d×%d", tc.iw, tc.ih, tc.cols, tc.rows, w, h, tc.w, tc.h)
		}
	}
}

func TestPreviewFitsLargeImage(t *testing.T) {
	img := gradient(300, 200)
	rows := renderPreview(img, 60, 12, tui.TrueColor)
	if len(rows) > 12 {
		t.Errorf("%d rows, box is 12", len(rows))
	}
	for _, r := range rows {
		if n := strings.Count(r, "▀"); n > 60 {
			t.Errorf("row of %d cells, box is 60", n)
		}
	}
	// 300×200 into 60×24 pixels: width limits, 60×40 would not fit, so 36×24.
	if got := strings.Count(rows[0], "▀"); got != 36 {
		t.Errorf("first row has %d cells, want 36", got)
	}
}

func TestANSI256Gray(t *testing.T) {
	for in, want := range map[[3]int]int{
		{0, 0, 0}: 16, {255, 255, 255}: 231, {128, 128, 128}: 244, {8, 8, 8}: 232, {255, 0, 0}: 196,
	} {
		if got := ansi256(in); got != want {
			t.Errorf("ansi256(%v) = %d, want %d", in, got, want)
		}
	}
}
