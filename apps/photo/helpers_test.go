package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"image"
	"image/png"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/sdk-go/tools"
)

// memAssets is an asset store on a temporary directory, with the daemon's
// content-addressed handles.
type memAssets struct {
	dir    string
	assets map[string]protocol.Asset
	puts   int
}

func newMemAssets(t *testing.T) *memAssets {
	return &memAssets{dir: t.TempDir(), assets: map[string]protocol.Asset{}}
}

func (m *memAssets) PutAsset(_ context.Context, put protocol.AssetPut) (protocol.Asset, error) {
	m.puts++
	var data []byte
	name := ""
	var err error
	switch {
	case put.Path != nil:
		data, err = os.ReadFile(*put.Path)
		name = filepath.Base(*put.Path)
	case put.Data != nil:
		data, err = base64.StdEncoding.DecodeString(*put.Data)
	}
	if err != nil {
		return protocol.Asset{}, err
	}
	if put.Name != nil {
		name = *put.Name
	}
	mime := http.DetectContentType(data)
	if put.Mime != nil {
		mime = *put.Mime
	}
	sum := sha256.Sum256(data)
	handle := "asset://img-" + hex.EncodeToString(sum[:])[:12]
	if a, ok := m.assets[handle]; ok {
		return a, nil
	}
	path := filepath.Join(m.dir, strings.TrimPrefix(handle, "asset://")+filepath.Ext(name))
	if err := os.WriteFile(path, data, 0o400); err != nil {
		return protocol.Asset{}, err
	}
	a := protocol.Asset{Handle: handle, Path: path, Name: name, Mime: mime, Size: int64(len(data))}
	m.assets[handle] = a
	return a, nil
}

func (m *memAssets) GetAsset(_ context.Context, handle string) (protocol.Asset, error) {
	if a, ok := m.assets[handle]; ok {
		return a, nil
	}
	return protocol.Asset{}, fmt.Errorf("no asset %s", handle)
}

// writePNG writes a w×h gradient PNG named name into dir and returns its path.
func writePNG(t *testing.T, dir, name string, w, h int) string {
	t.Helper()
	var b bytes.Buffer
	if err := png.Encode(&b, gradient(w, h)); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, b.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

// assetSize decodes the stored file behind an asset and returns its size.
func assetSize(t *testing.T, path string) (int, int) {
	t.Helper()
	f, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	cfg, _, err := image.DecodeConfig(f)
	if err != nil {
		t.Fatal(err)
	}
	return cfg.Width, cfg.Height
}

func toolByName(t *testing.T, list []tools.Tool, name string) tools.Tool {
	t.Helper()
	for _, tool := range list {
		if tool.Name == name {
			return tool
		}
	}
	t.Fatalf("no tool %s", name)
	return tools.Tool{}
}

func putData(text, name, mime string) protocol.AssetPut {
	enc := base64.StdEncoding.EncodeToString([]byte(text))
	return protocol.AssetPut{Data: &enc, Name: &name, Mime: &mime}
}

func putPath(path string) protocol.AssetPut { return protocol.AssetPut{Path: &path} }
