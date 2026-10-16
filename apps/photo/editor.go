package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"image"
	"os"
	"path/filepath"
	"strings"
	"sync"

	protocol "neoworks.dev/suite/protocol/go"
)

// maxHistory is how many images undo keeps in memory.
const maxHistory = 20

// assetStore is the part of the daemon connection the editor needs.
type assetStore interface {
	PutAsset(ctx context.Context, put protocol.AssetPut) (protocol.Asset, error)
	GetAsset(ctx context.Context, handle string) (protocol.Asset, error)
}

// doc is one state of the image being edited: its pixels and the asset that
// holds them (so earlier states stay addressable by their handles).
type doc struct {
	img   *image.NRGBA
	asset protocol.Asset
	base  string // name without extension, kept through edits
}

// imageResult is what every tool returns as structuredContent.
type imageResult struct {
	Handle string `json:"handle"`
	Name   string `json:"name"`
	Width  int    `json:"width"`
	Height int    `json:"height"`
	Mime   string `json:"mime"`
}

func (d *doc) result() imageResult {
	b := d.img.Bounds()
	return imageResult{Handle: d.asset.Handle, Name: d.asset.Name, Width: b.Dx(), Height: b.Dy(), Mime: d.asset.Mime}
}

var errNoImage = errors.New("no image is open; call open with an asset handle or a file path first")

// editor holds "the current image" and the undo history. Every edit stores
// its result as a new asset, so handles handed out earlier stay valid.
type editor struct {
	assets assetStore

	mu      sync.Mutex
	history []*doc // oldest first; the last one is current
	changed chan struct{}
	version int // counts changes, so the UI knows when to redraw
}

func newEditor(assets assetStore) *editor {
	return &editor{assets: assets, changed: make(chan struct{}, 1)}
}

// notifyLocked wakes whoever waits on changed.
func (e *editor) notifyLocked() {
	e.version++
	select {
	case e.changed <- struct{}{}:
	default:
	}
}

// current returns the image being edited, or nil.
func (e *editor) current() (d *doc, version int) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if len(e.history) == 0 {
		return nil, e.version
	}
	return e.history[len(e.history)-1], e.version
}

// openAsset loads the image behind an asset handle.
func (e *editor) openAsset(ctx context.Context, handle string) (imageResult, error) {
	asset, err := e.assets.GetAsset(ctx, handle)
	if err != nil {
		return imageResult{}, fmt.Errorf("asset %s: %w", handle, err)
	}
	if !strings.HasPrefix(asset.Mime, "image/") {
		return imageResult{}, fmt.Errorf("%s is %s, not an image", handle, asset.Mime)
	}
	img, err := readImage(asset.Path)
	if err != nil {
		return imageResult{}, fmt.Errorf("%s: %w", handle, err)
	}
	return e.start(&doc{img: img, asset: asset, base: stem(asset.Name)}), nil
}

// openPath loads a local image file and stores it as an asset.
func (e *editor) openPath(ctx context.Context, path string) (imageResult, error) {
	if !filepath.IsAbs(path) {
		abs, err := filepath.Abs(path)
		if err != nil {
			return imageResult{}, err
		}
		path = abs
	}
	img, err := readImage(path)
	if err != nil {
		return imageResult{}, err
	}
	asset, err := e.assets.PutAsset(ctx, protocol.AssetPut{Path: &path})
	if err != nil {
		return imageResult{}, fmt.Errorf("storing %s: %w", path, err)
	}
	return e.start(&doc{img: img, asset: asset, base: stem(asset.Name)}), nil
}

func readImage(path string) (*image.NRGBA, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	return decodeImage(f)
}

// start makes d the only state: opening an image forgets the undo history.
func (e *editor) start(d *doc) imageResult {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.history = []*doc{d}
	e.notifyLocked()
	return d.result()
}

// info describes the current image.
func (e *editor) info() (imageResult, error) {
	d, _ := e.current()
	if d == nil {
		return imageResult{}, errNoImage
	}
	return d.result(), nil
}

// edit runs op on the current image and stores the result as a new asset and
// the new current image. op names the edit in the new asset's file name.
func (e *editor) edit(ctx context.Context, op string, fn func(src *image.NRGBA) (*image.NRGBA, error)) (imageResult, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if len(e.history) == 0 {
		return imageResult{}, errNoImage
	}
	cur := e.history[len(e.history)-1]
	out, err := fn(cur.img)
	if err != nil {
		return imageResult{}, err
	}
	data, err := encodePNG(out)
	if err != nil {
		return imageResult{}, err
	}
	asset, err := e.store(ctx, data, cur.base+"-"+op+".png", "image/png")
	if err != nil {
		return imageResult{}, err
	}
	next := &doc{img: out, asset: asset, base: cur.base}
	e.history = append(e.history, next)
	if len(e.history) > maxHistory {
		e.history = e.history[len(e.history)-maxHistory:]
	}
	e.notifyLocked()
	return next.result(), nil
}

// undo goes back to the previous image. It makes no new asset: the earlier
// one is still in the store.
func (e *editor) undo() (imageResult, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if len(e.history) == 0 {
		return imageResult{}, errNoImage
	}
	if len(e.history) == 1 {
		return imageResult{}, errors.New("nothing to undo")
	}
	e.history = e.history[:len(e.history)-1]
	e.notifyLocked()
	return e.history[len(e.history)-1].result(), nil
}

// export encodes the current image as png or jpeg and stores it as an
// asset. The current image stays as it was.
func (e *editor) export(ctx context.Context, format string, quality int) (imageResult, error) {
	d, _ := e.current()
	if d == nil {
		return imageResult{}, errNoImage
	}
	var data []byte
	var err error
	var ext, mime string
	switch format {
	case "png":
		data, err = encodePNG(d.img)
		ext, mime = ".png", "image/png"
	case "jpeg", "jpg":
		if quality < 1 || quality > 100 {
			return imageResult{}, fmt.Errorf("quality must be between 1 and 100 (got %d)", quality)
		}
		data, err = encodeJPEG(d.img, quality)
		ext, mime = ".jpg", "image/jpeg"
	default:
		return imageResult{}, fmt.Errorf(`format must be "png" or "jpeg" (got %q)`, format)
	}
	if err != nil {
		return imageResult{}, err
	}
	asset, err := e.store(ctx, data, d.base+ext, mime)
	if err != nil {
		return imageResult{}, err
	}
	res := d.result()
	res.Handle, res.Name, res.Mime = asset.Handle, asset.Name, asset.Mime
	return res, nil
}

func (e *editor) store(ctx context.Context, data []byte, name, mime string) (protocol.Asset, error) {
	enc := base64.StdEncoding.EncodeToString(data)
	asset, err := e.assets.PutAsset(ctx, protocol.AssetPut{Data: &enc, Name: &name, Mime: &mime})
	if err != nil {
		return protocol.Asset{}, fmt.Errorf("storing %s: %w", name, err)
	}
	return asset, nil
}

// stem is a file name without its directory and extension.
func stem(name string) string {
	name = filepath.Base(name)
	if s := strings.TrimSuffix(name, filepath.Ext(name)); s != "" && s != "." {
		return s
	}
	return "image"
}
