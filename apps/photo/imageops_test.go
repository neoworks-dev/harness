package main

import (
	"bytes"
	"image"
	"image/color"
	"image/gif"
	"image/jpeg"
	"image/png"
	"strings"
	"testing"
)

// gradient is a w×h image whose pixel (x, y) has red x, green y and blue 7,
// so every pixel is distinct and its origin recognizable.
func gradient(w, h int) *image.NRGBA {
	img := image.NewNRGBA(image.Rect(0, 0, w, h))
	for y := range h {
		for x := range w {
			img.SetNRGBA(x, y, color.NRGBA{uint8(x), uint8(y), 7, 255})
		}
	}
	return img
}

func dims(img image.Image) (int, int) { return img.Bounds().Dx(), img.Bounds().Dy() }

func TestCrop(t *testing.T) {
	src := gradient(10, 8)
	out, err := crop(src, 2, 3, 5, 4)
	if err != nil {
		t.Fatal(err)
	}
	if w, h := dims(out); w != 5 || h != 4 {
		t.Fatalf("got %d×%d", w, h)
	}
	if got := out.NRGBAAt(0, 0); got != (color.NRGBA{2, 3, 7, 255}) {
		t.Errorf("top-left pixel %v", got)
	}
	if got := out.NRGBAAt(4, 3); got != (color.NRGBA{6, 6, 7, 255}) {
		t.Errorf("bottom-right pixel %v", got)
	}
	// The whole image and a single pixel at the far corner are fine.
	if _, err := crop(src, 0, 0, 10, 8); err != nil {
		t.Error(err)
	}
	if out, err := crop(src, 9, 7, 1, 1); err != nil || out.NRGBAAt(0, 0) != (color.NRGBA{9, 7, 7, 255}) {
		t.Errorf("corner: %v %v", out, err)
	}
}

func TestCropBounds(t *testing.T) {
	src := gradient(10, 8)
	for name, c := range map[string][4]int{
		"too wide":       {5, 0, 6, 1},
		"too tall":       {0, 5, 1, 4},
		"negative x":     {-1, 0, 2, 2},
		"negative y":     {0, -1, 2, 2},
		"zero width":     {0, 0, 0, 2},
		"zero height":    {0, 0, 2, 0},
		"outside":        {10, 0, 1, 1},
		"negative width": {5, 5, -2, 1},
	} {
		if out, err := crop(src, c[0], c[1], c[2], c[3]); err == nil {
			t.Errorf("%s: accepted, got %v", name, out.Bounds())
		}
	}
	_, err := crop(src, 8, 0, 5, 1)
	if err == nil || !strings.Contains(err.Error(), "10×8") {
		t.Errorf("the error should name the image size: %v", err)
	}
}

func TestResizeKeepsAspect(t *testing.T) {
	for _, tc := range []struct{ iw, ih, w, h, wantW, wantH int }{
		{400, 200, 100, 0, 100, 50},
		{400, 200, 0, 100, 200, 100},
		{400, 200, 300, 0, 300, 150},
		{3, 1000, 0, 10, 1, 10}, // never below one pixel
		{400, 200, 50, 70, 50, 70},
		{10, 10, 40, 0, 40, 40},
	} {
		w, h, err := fitSize(tc.iw, tc.ih, tc.w, tc.h)
		if err != nil || w != tc.wantW || h != tc.wantH {
			t.Errorf("fitSize(%d×%d, %d, %d) = %d×%d, %v; want %d×%d", tc.iw, tc.ih, tc.w, tc.h, w, h, err, tc.wantW, tc.wantH)
		}
	}
	for _, tc := range [][2]int{{0, 0}, {-1, 5}, {5, -1}, {maxSide + 1, 0}, {40000, 40000}} {
		if _, _, err := fitSize(400, 200, tc[0], tc[1]); err == nil {
			t.Errorf("fitSize(400×200, %d, %d) accepted", tc[0], tc[1])
		}
	}
}

func TestResizePixels(t *testing.T) {
	src := image.NewNRGBA(image.Rect(0, 0, 4, 2))
	for x := range 4 {
		v := uint8(0)
		if x >= 2 {
			v = 200
		}
		for y := range 2 {
			src.SetNRGBA(x, y, color.NRGBA{v, v, v, 255})
		}
	}
	// Shrinking averages: a 2×1 image of the two halves.
	small := resize(src, 2, 1)
	if w, h := dims(small); w != 2 || h != 1 {
		t.Fatalf("got %d×%d", w, h)
	}
	if small.NRGBAAt(0, 0).R != 0 || small.NRGBAAt(1, 0).R != 200 {
		t.Errorf("shrunk: %v %v", small.NRGBAAt(0, 0), small.NRGBAAt(1, 0))
	}
	// Enlarging interpolates: the edge pixels keep their values, the middle is in between.
	big := resize(src, 8, 4)
	if w, h := dims(big); w != 8 || h != 4 {
		t.Fatalf("got %d×%d", w, h)
	}
	if big.NRGBAAt(0, 0).R != 0 || big.NRGBAAt(7, 3).R != 200 {
		t.Errorf("edges: %v %v", big.NRGBAAt(0, 0), big.NRGBAAt(7, 3))
	}
	if mid := big.NRGBAAt(3, 1).R; mid == 0 || mid >= 200 {
		t.Errorf("middle pixel %d should be between the halves", mid)
	}
	// Same size is a copy.
	if same := resize(src, 4, 2); same == src || same.NRGBAAt(3, 1) != src.NRGBAAt(3, 1) {
		t.Error("same-size resize")
	}
}

func TestResizeTransparencyDoesNotBleed(t *testing.T) {
	src := image.NewNRGBA(image.Rect(0, 0, 2, 1))
	src.SetNRGBA(0, 0, color.NRGBA{255, 0, 0, 255})
	src.SetNRGBA(1, 0, color.NRGBA{0, 255, 0, 0}) // invisible green
	out := resize(src, 1, 1)
	if got := out.NRGBAAt(0, 0); got != (color.NRGBA{255, 0, 0, 127}) {
		t.Errorf("got %v", got)
	}
}

func TestRotate(t *testing.T) {
	src := gradient(5, 3) // pixel (x, y) = (x, y, 7)
	for _, tc := range []struct {
		degrees, w, h int
		// where the source's top-left pixel (0, 0) and top-right (4, 0) end up
		topLeft, topRight image.Point
	}{
		{90, 3, 5, image.Pt(2, 0), image.Pt(2, 4)},
		{180, 5, 3, image.Pt(4, 2), image.Pt(0, 2)},
		{270, 3, 5, image.Pt(0, 4), image.Pt(0, 0)},
		{-90, 3, 5, image.Pt(0, 4), image.Pt(0, 0)},
	} {
		out, err := rotate(src, tc.degrees)
		if err != nil {
			t.Fatalf("%d: %v", tc.degrees, err)
		}
		if w, h := dims(out); w != tc.w || h != tc.h {
			t.Errorf("%d: got %d×%d, want %d×%d", tc.degrees, w, h, tc.w, tc.h)
		}
		if got := out.NRGBAAt(tc.topLeft.X, tc.topLeft.Y); got.R != 0 || got.G != 0 {
			t.Errorf("%d: source (0,0) not at %v: %v", tc.degrees, tc.topLeft, got)
		}
		if got := out.NRGBAAt(tc.topRight.X, tc.topRight.Y); got.R != 4 || got.G != 0 {
			t.Errorf("%d: source (4,0) not at %v: %v", tc.degrees, tc.topRight, got)
		}
	}
	// Four quarter turns are the identity.
	out := src
	for range 4 {
		out, _ = rotate(out, 90)
	}
	if !bytes.Equal(out.Pix, src.Pix) {
		t.Error("four turns did not restore the image")
	}
	for _, bad := range []int{0, 45, 360} {
		if _, err := rotate(src, bad); err == nil {
			t.Errorf("%d degrees accepted", bad)
		}
	}
}

func TestFlip(t *testing.T) {
	src := gradient(4, 3)
	h, err := flip(src, "horizontal")
	if err != nil {
		t.Fatal(err)
	}
	if got := h.NRGBAAt(0, 1); got != (color.NRGBA{3, 1, 7, 255}) {
		t.Errorf("horizontal: %v", got)
	}
	v, _ := flip(src, "vertical")
	if got := v.NRGBAAt(1, 0); got != (color.NRGBA{1, 2, 7, 255}) {
		t.Errorf("vertical: %v", got)
	}
	for _, out := range []*image.NRGBA{h, v} {
		if w, hh := dims(out); w != 4 || hh != 3 {
			t.Errorf("size %d×%d", w, hh)
		}
	}
	if _, err := flip(src, "diagonal"); err == nil {
		t.Error("diagonal accepted")
	}
}

func TestGrayscale(t *testing.T) {
	src := image.NewNRGBA(image.Rect(0, 0, 3, 1))
	src.SetNRGBA(0, 0, color.NRGBA{255, 0, 0, 255})
	src.SetNRGBA(1, 0, color.NRGBA{255, 255, 255, 255})
	src.SetNRGBA(2, 0, color.NRGBA{0, 0, 255, 90})
	out := grayscale(src)
	for x := range 3 {
		if p := out.NRGBAAt(x, 0); p.R != p.G || p.G != p.B {
			t.Errorf("pixel %d not gray: %v", x, p)
		}
	}
	if out.NRGBAAt(0, 0).R != 76 || out.NRGBAAt(1, 0).R != 255 {
		t.Errorf("luminance: %v %v", out.NRGBAAt(0, 0), out.NRGBAAt(1, 0))
	}
	if out.NRGBAAt(2, 0).A != 90 {
		t.Errorf("alpha lost: %v", out.NRGBAAt(2, 0))
	}
}

func encoded(t *testing.T, format string) []byte {
	t.Helper()
	img := gradient(6, 4)
	var b bytes.Buffer
	var err error
	switch format {
	case "png":
		err = png.Encode(&b, img)
	case "jpeg":
		err = jpeg.Encode(&b, img, nil)
	case "gif":
		pal := image.NewPaletted(img.Bounds(), color.Palette{color.Black, color.White})
		err = gif.Encode(&b, pal, nil)
	}
	if err != nil {
		t.Fatal(err)
	}
	return b.Bytes()
}

func TestDecodeFormats(t *testing.T) {
	for _, format := range []string{"png", "jpeg", "gif"} {
		img, err := decodeImage(bytes.NewReader(encoded(t, format)))
		if err != nil {
			t.Errorf("%s: %v", format, err)
			continue
		}
		if w, h := dims(img); w != 6 || h != 4 || img.Bounds().Min != (image.Point{}) {
			t.Errorf("%s: %v", format, img.Bounds())
		}
	}
}

func TestDecodeRejects(t *testing.T) {
	webp := append([]byte("RIFF\x00\x00\x00\x00WEBPVP8 "), make([]byte, 20)...)
	if _, err := decodeImage(bytes.NewReader(webp)); err == nil || !strings.Contains(err.Error(), "WebP") {
		t.Errorf("webp: %v", err)
	}
	if _, err := decodeImage(strings.NewReader("hello")); err == nil || !strings.Contains(err.Error(), "not an image") {
		t.Errorf("text: %v", err)
	}
	// A header claiming a huge size is refused before any pixels are allocated.
	huge := encoded(t, "png")
	huge[16], huge[17], huge[18], huge[19] = 0x00, 0x01, 0x00, 0x00 // width 65536
	if _, err := decodeImage(bytes.NewReader(huge)); err == nil {
		t.Error("oversized image accepted")
	}
}

func TestEncodeJPEGFlattensAlpha(t *testing.T) {
	src := image.NewNRGBA(image.Rect(0, 0, 8, 8)) // fully transparent
	data, err := encodeJPEG(src, 90)
	if err != nil {
		t.Fatal(err)
	}
	img, err := jpeg.Decode(bytes.NewReader(data))
	if err != nil {
		t.Fatal(err)
	}
	if r, g, b, _ := img.At(4, 4).RGBA(); r>>8 < 250 || g>>8 < 250 || b>>8 < 250 {
		t.Errorf("transparent area should become white, got %d %d %d", r>>8, g>>8, b>>8)
	}
}
