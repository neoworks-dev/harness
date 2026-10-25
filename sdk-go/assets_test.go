package sdk_test

import (
	"context"
	"encoding/base64"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	protocol "neoworks.dev/neo/protocol/go"
	sdk "neoworks.dev/neo/sdk-go"
	"neoworks.dev/neo/sdk-go/sdktest"
)

var pngMagic = []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0}

// checkAssets exercises PutAsset and GetAsset against whichever daemon the
// client is connected to.
func checkAssets(t *testing.T, c *sdk.Client) {
	t.Helper()
	ctx := ctxT(t)

	data := base64.StdEncoding.EncodeToString(pngMagic)
	a, err := c.PutAsset(ctx, protocol.AssetPut{Data: &data, Name: ptr("dot.png")})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(a.Handle, "asset://img-") || a.Name != "dot.png" || a.Mime != "image/png" || a.Size != int64(len(pngMagic)) {
		t.Fatalf("put data: %+v", a)
	}
	if a.App == nil || *a.App != "asset-test" {
		t.Errorf("app %v", a.App)
	}
	got, err := os.ReadFile(a.Path)
	if err != nil || string(got) != string(pngMagic) {
		t.Fatalf("stored file: %v %q", err, got)
	}

	// Same bytes from a path: same handle.
	src := filepath.Join(t.TempDir(), "copy.png")
	if err := os.WriteFile(src, pngMagic, 0o600); err != nil {
		t.Fatal(err)
	}
	b, err := c.PutAsset(ctx, protocol.AssetPut{Path: &src})
	if err != nil {
		t.Fatal(err)
	}
	if b.Handle != a.Handle {
		t.Errorf("handles differ: %s vs %s", b.Handle, a.Handle)
	}

	info, err := c.GetAsset(ctx, a.Handle)
	if err != nil || info.Handle != a.Handle || info.Path != a.Path || info.Mime != "image/png" {
		t.Fatalf("get: %v %+v", err, info)
	}

	_, err = c.GetAsset(ctx, "asset://img-000000000000")
	var refused *sdk.Error
	if !errors.As(err, &refused) || refused.Code != protocol.ErrorCodeNotFound {
		t.Errorf("unknown handle: %v", err)
	}
	if _, err := c.PutAsset(ctx, protocol.AssetPut{}); !errors.As(err, &refused) || refused.Code != protocol.ErrorCodeBadRequest {
		t.Errorf("empty put: %v", err)
	}
}

func TestAssetsFake(t *testing.T) {
	d := sdktest.New(t)
	checkAssets(t, dial(t, d, sdk.Options{Client: "asset-test"}))
}

func TestAssetsRealDaemon(t *testing.T) {
	d := sdktest.StartReal(t)
	c, err := sdk.Dial(ctxT(t), sdk.Options{Socket: d.Socket, Client: "asset-test"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	checkAssets(t, c)
}

type changes struct {
	mu   sync.Mutex
	list []protocol.FileChanged
}

func (c *changes) add(f protocol.FileChanged) {
	c.mu.Lock()
	c.list = append(c.list, f)
	c.mu.Unlock()
}

func (c *changes) get() []protocol.FileChanged {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]protocol.FileChanged(nil), c.list...)
}

func checkFileChanged(t *testing.T, dialer func(name string, onChange func(protocol.FileChanged)) *sdk.Client) {
	t.Helper()
	var seenByB, seenByA changes
	a := dialer("editor-a", seenByA.add)
	_ = dialer("editor-b", seenByB.add)

	if err := a.FileChanged(context.Background(), "/tmp/x/logo.png"); err != nil {
		t.Fatal(err)
	}
	sdktest.Eventually(t, "the other client hears about the change", func() bool { return len(seenByB.get()) == 1 })
	got := seenByB.get()[0]
	if got.Path != "/tmp/x/logo.png" || got.App == nil || *got.App != "editor-a" {
		t.Errorf("file.changed: %+v", got)
	}
	// Not echoed to the sender.
	if n := len(seenByA.get()); n != 0 {
		t.Errorf("sender got %d echoes", n)
	}
}

func TestFileChangedFake(t *testing.T) {
	d := sdktest.New(t)
	checkFileChanged(t, func(name string, on func(protocol.FileChanged)) *sdk.Client {
		return dial(t, d, sdk.Options{Client: name, OnFileChanged: on})
	})
	// A broadcast from the daemon itself (the agent wrote a file).
	var seen changes
	dial(t, d, sdk.Options{Client: "viewer", OnFileChanged: seen.add})
	d.SendFileChanged(protocol.FileChanged{Path: "/p/a.go", App: ptr("agent"), Session: ptr("s1")})
	sdktest.Eventually(t, "the daemon's file.changed arrives", func() bool { return len(seen.get()) == 1 })
	if f := seen.get()[0]; f.Path != "/p/a.go" || *f.App != "agent" || *f.Session != "s1" {
		t.Errorf("%+v", f)
	}
}

func TestFileChangedRealDaemon(t *testing.T) {
	d := sdktest.StartReal(t)
	checkFileChanged(t, func(name string, on func(protocol.FileChanged)) *sdk.Client {
		c, err := sdk.Dial(ctxT(t), sdk.Options{Socket: d.Socket, Client: name, OnFileChanged: on})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { c.Close() })
		return c
	})
}
