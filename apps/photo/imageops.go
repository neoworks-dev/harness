package main

import (
	"bytes"
	"errors"
	"fmt"
	"image"
	"image/color"
	"image/draw"
	_ "image/gif" // registers GIF for image.Decode; the editor never writes one
	"image/jpeg"
	"image/png"
	"io"
	"math"
)

// Limits keep one request from exhausting memory: an NRGBA image takes four
// bytes a pixel.
const (
	maxSide   = 20000
	maxPixels = 50_000_000
)

// toNRGBA copies any image into a non-premultiplied image whose bounds start
// at the origin, which is what every operation here works on.
func toNRGBA(src image.Image) *image.NRGBA {
	b := src.Bounds()
	dst := image.NewNRGBA(image.Rect(0, 0, b.Dx(), b.Dy()))
	draw.Draw(dst, dst.Bounds(), src, b.Min, draw.Src)
	return dst
}

// decodeImage reads a PNG, JPEG or GIF (its first frame). WebP and anything
// else is an error that says so.
func decodeImage(r io.Reader) (*image.NRGBA, error) {
	data, err := io.ReadAll(io.LimitReader(r, 512<<20))
	if err != nil {
		return nil, err
	}
	cfg, format, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		if errors.Is(err, image.ErrFormat) {
			if len(data) >= 12 && string(data[:4]) == "RIFF" && string(data[8:12]) == "WEBP" {
				return nil, errors.New("WebP images are not supported (PNG, JPEG and GIF are)")
			}
			return nil, errors.New("not an image this editor can read (PNG, JPEG and GIF are supported)")
		}
		return nil, err
	}
	if err := checkSize(cfg.Width, cfg.Height); err != nil {
		return nil, fmt.Errorf("the %s is too large: %v", format, err)
	}
	img, _, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	return toNRGBA(img), nil
}

// checkSize rejects dimensions that are empty or too large.
func checkSize(w, h int) error {
	switch {
	case w < 1 || h < 1:
		return fmt.Errorf("%d×%d has no pixels", w, h)
	case w > maxSide || h > maxSide:
		return fmt.Errorf("%d×%d exceeds %d pixels on a side", w, h, maxSide)
	case w*h > maxPixels:
		return fmt.Errorf("%d×%d exceeds %d megapixels", w, h, maxPixels/1_000_000)
	}
	return nil
}

func encodePNG(img image.Image) ([]byte, error) {
	var b bytes.Buffer
	if err := png.Encode(&b, img); err != nil {
		return nil, err
	}
	return b.Bytes(), nil
}

// encodeJPEG flattens transparency onto white first, as JPEG has none.
func encodeJPEG(img *image.NRGBA, quality int) ([]byte, error) {
	flat := image.NewRGBA(img.Bounds())
	draw.Draw(flat, flat.Bounds(), image.NewUniform(color.White), image.Point{}, draw.Src)
	draw.Draw(flat, flat.Bounds(), img, image.Point{}, draw.Over)
	var b bytes.Buffer
	if err := jpeg.Encode(&b, flat, &jpeg.Options{Quality: quality}); err != nil {
		return nil, err
	}
	return b.Bytes(), nil
}

// crop returns the w×h region whose top-left corner is (x, y).
func crop(src *image.NRGBA, x, y, w, h int) (*image.NRGBA, error) {
	b := src.Bounds()
	switch {
	case w < 1 || h < 1:
		return nil, fmt.Errorf("width and height must be at least 1 (got %d×%d)", w, h)
	case x < 0 || y < 0:
		return nil, fmt.Errorf("x and y must not be negative (got %d, %d)", x, y)
	case x+w > b.Dx() || y+h > b.Dy():
		return nil, fmt.Errorf("the region %d×%d at (%d, %d) reaches outside the %d×%d image", w, h, x, y, b.Dx(), b.Dy())
	}
	dst := image.NewNRGBA(image.Rect(0, 0, w, h))
	for row := range h {
		from := src.PixOffset(x, y+row)
		copy(dst.Pix[row*dst.Stride:row*dst.Stride+w*4], src.Pix[from:from+w*4])
	}
	return dst, nil
}

// rotate turns the image clockwise by 90, 180 or 270 degrees.
func rotate(src *image.NRGBA, degrees int) (*image.NRGBA, error) {
	w, h := src.Bounds().Dx(), src.Bounds().Dy()
	var dst *image.NRGBA
	var at func(x, y int) (int, int) // where source pixel (x, y) goes
	switch ((degrees % 360) + 360) % 360 {
	case 90:
		dst = image.NewNRGBA(image.Rect(0, 0, h, w))
		at = func(x, y int) (int, int) { return h - 1 - y, x }
	case 180:
		dst = image.NewNRGBA(image.Rect(0, 0, w, h))
		at = func(x, y int) (int, int) { return w - 1 - x, h - 1 - y }
	case 270:
		dst = image.NewNRGBA(image.Rect(0, 0, h, w))
		at = func(x, y int) (int, int) { return y, w - 1 - x }
	default:
		return nil, fmt.Errorf("degrees must be 90, 180 or 270 (got %d)", degrees)
	}
	for y := range h {
		for x := range w {
			dx, dy := at(x, y)
			copy(dst.Pix[dst.PixOffset(dx, dy):][:4], src.Pix[src.PixOffset(x, y):][:4])
		}
	}
	return dst, nil
}

// flip mirrors the image: "horizontal" swaps left and right, "vertical" top
// and bottom.
func flip(src *image.NRGBA, direction string) (*image.NRGBA, error) {
	w, h := src.Bounds().Dx(), src.Bounds().Dy()
	dst := image.NewNRGBA(src.Bounds())
	switch direction {
	case "horizontal":
		for y := range h {
			for x := range w {
				copy(dst.Pix[dst.PixOffset(w-1-x, y):][:4], src.Pix[src.PixOffset(x, y):][:4])
			}
		}
	case "vertical":
		for y := range h {
			copy(dst.Pix[dst.PixOffset(0, h-1-y):][:w*4], src.Pix[src.PixOffset(0, y):][:w*4])
		}
	default:
		return nil, fmt.Errorf(`direction must be "horizontal" or "vertical" (got %q)`, direction)
	}
	return dst, nil
}

// grayscale replaces the colors by their luminance and keeps the alpha.
func grayscale(src *image.NRGBA) *image.NRGBA {
	dst := image.NewNRGBA(src.Bounds())
	for i := 0; i+3 < len(src.Pix); i += 4 {
		r, g, b := int(src.Pix[i]), int(src.Pix[i+1]), int(src.Pix[i+2])
		y := uint8((299*r + 587*g + 114*b + 500) / 1000)
		dst.Pix[i], dst.Pix[i+1], dst.Pix[i+2], dst.Pix[i+3] = y, y, y, src.Pix[i+3]
	}
	return dst
}

// fitSize completes a target size: with one side missing (0) the other
// decides and the aspect ratio is kept; with both, they are taken as given.
func fitSize(iw, ih, w, h int) (int, int, error) {
	switch {
	case w < 0 || h < 0:
		return 0, 0, errors.New("width and height must be positive")
	case w == 0 && h == 0:
		return 0, 0, errors.New("give a width, a height, or both")
	case h == 0:
		h = max(1, int(math.Round(float64(ih)*float64(w)/float64(iw))))
	case w == 0:
		w = max(1, int(math.Round(float64(iw)*float64(h)/float64(ih))))
	}
	if err := checkSize(w, h); err != nil {
		return 0, 0, err
	}
	return w, h, nil
}

// resize scales to w×h: averaging when the result is smaller, bilinear
// interpolation when it is larger.
func resize(src *image.NRGBA, w, h int) *image.NRGBA {
	iw, ih := src.Bounds().Dx(), src.Bounds().Dy()
	if w == iw && h == ih {
		return toNRGBA(src)
	}
	if w <= iw && h <= ih {
		return areaResize(src, w, h)
	}
	return bilinearResize(src, w, h)
}

// areaResize averages the source pixels under each target pixel (w ≤ source
// width, h ≤ source height). Color is weighted by alpha so transparent pixels
// do not bleed their color.
func areaResize(src *image.NRGBA, w, h int) *image.NRGBA {
	iw, ih := src.Bounds().Dx(), src.Bounds().Dy()
	dst := image.NewNRGBA(image.Rect(0, 0, w, h))
	for y := range h {
		y0, y1 := y*ih/h, max((y+1)*ih/h, y*ih/h+1)
		for x := range w {
			x0, x1 := x*iw/w, max((x+1)*iw/w, x*iw/w+1)
			var r, g, b, a, n int
			for sy := y0; sy < y1; sy++ {
				row := src.Pix[src.PixOffset(x0, sy):]
				for sx := 0; sx < x1-x0; sx++ {
					pa := int(row[sx*4+3])
					r += int(row[sx*4]) * pa
					g += int(row[sx*4+1]) * pa
					b += int(row[sx*4+2]) * pa
					a += pa
					n++
				}
			}
			o := dst.PixOffset(x, y)
			if a > 0 {
				dst.Pix[o], dst.Pix[o+1], dst.Pix[o+2] = uint8(r/a), uint8(g/a), uint8(b/a)
			}
			dst.Pix[o+3] = uint8(a / n)
		}
	}
	return dst
}

// bilinearResize interpolates between the four source pixels around each
// target pixel's center, weighting color by alpha.
func bilinearResize(src *image.NRGBA, w, h int) *image.NRGBA {
	iw, ih := src.Bounds().Dx(), src.Bounds().Dy()
	dst := image.NewNRGBA(image.Rect(0, 0, w, h))
	for y := range h {
		fy := (float64(y)+0.5)*float64(ih)/float64(h) - 0.5
		y0 := int(math.Floor(fy))
		ty := fy - float64(y0)
		for x := range w {
			fx := (float64(x)+0.5)*float64(iw)/float64(w) - 0.5
			x0 := int(math.Floor(fx))
			tx := fx - float64(x0)
			var r, g, b, a float64
			for _, s := range [4]struct {
				dx, dy int
				w      float64
			}{
				{0, 0, (1 - tx) * (1 - ty)}, {1, 0, tx * (1 - ty)},
				{0, 1, (1 - tx) * ty}, {1, 1, tx * ty},
			} {
				px, py := min(max(x0+s.dx, 0), iw-1), min(max(y0+s.dy, 0), ih-1)
				p := src.Pix[src.PixOffset(px, py):][:4]
				wa := s.w * float64(p[3])
				r += wa * float64(p[0])
				g += wa * float64(p[1])
				b += wa * float64(p[2])
				a += wa
			}
			o := dst.PixOffset(x, y)
			if a > 0 {
				dst.Pix[o], dst.Pix[o+1], dst.Pix[o+2] = round8(r/a), round8(g/a), round8(b/a)
			}
			dst.Pix[o+3] = round8(a)
		}
	}
	return dst
}

func round8(v float64) uint8 { return uint8(min(max(math.Round(v), 0), 255)) }
