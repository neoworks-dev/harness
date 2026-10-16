package sdktest

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"
)

// RealDaemon is the TypeScript daemon (`bun run daemon/src/main.ts`) started
// with the scripted echo agent and a temporary socket and data directory.
type RealDaemon struct {
	// Socket is the bus socket clients dial.
	Socket string
	// DataDir holds the asset store (assets/) and the session database.
	DataDir string
	// URL is the daemon's loopback HTTP address (/mcp, /assets).
	URL string

	token string // the bearer token for URL; never printed
	t     testing.TB
	out   *lockedBuffer
}

// StartReal starts the real daemon for a test and stops it when the test
// ends. It skips the test when bun is not installed.
func StartReal(t testing.TB) *RealDaemon {
	t.Helper()
	bun, err := exec.LookPath("bun")
	if err != nil {
		t.Skip("bun is not installed; cannot start the real daemon")
	}
	root := repoRoot(t)
	dir, err := os.MkdirTemp("", "suite")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	r := &RealDaemon{
		Socket:  filepath.Join(dir, "bus.sock"),
		DataDir: filepath.Join(dir, "data"),
		t:       t,
		out:     &lockedBuffer{},
	}
	cmd := exec.Command(bun, "run", "daemon/src/main.ts")
	cmd.Dir = root
	cmd.Env = append(os.Environ(),
		"SUITE_AGENT=echo",
		"SUITE_SERVICES=none",
		"SUITE_SOCKET="+r.Socket,
		"SUITE_DATA_DIR="+r.DataDir,
		"SUITE_PORT=0",
	)
	cmd.Stdout, cmd.Stderr = r.out, r.out
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	exited := make(chan struct{})
	go func() { cmd.Wait(); close(exited) }()
	t.Cleanup(func() {
		cmd.Process.Signal(syscall.SIGTERM)
		select {
		case <-exited:
		case <-time.After(5 * time.Second):
			cmd.Process.Kill()
			<-exited
		}
	})

	info := filepath.Join(dir, "daemon.json")
	deadline := time.Now().Add(20 * time.Second)
	for {
		if data, err := os.ReadFile(info); err == nil {
			var f struct {
				URL   string `json:"url"`
				Token string `json:"token"`
			}
			if json.Unmarshal(data, &f) == nil && f.URL != "" {
				if c, err := net.Dial("unix", r.Socket); err == nil {
					c.Close()
					r.URL, r.token = f.URL, f.Token
					return r
				}
			}
		}
		select {
		case <-exited:
			t.Fatalf("daemon exited at startup:\n%s", r.out.String())
		case <-time.After(25 * time.Millisecond):
		}
		if time.Now().After(deadline) {
			t.Fatalf("daemon did not start:\n%s", r.out.String())
		}
	}
}

// Log is what the daemon printed so far (it never contains the token).
func (r *RealDaemon) Log() string { return r.out.String() }

// ToolResult is a tools/call result as the daemon's /mcp returns it.
type ToolResult struct {
	Content           []ToolContent
	StructuredContent map[string]any
	IsError           bool
}

// ToolContent is one content block of a ToolResult.
type ToolContent struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// Text joins the text blocks.
func (r ToolResult) Text() string {
	var parts []string
	for _, c := range r.Content {
		if c.Type == "text" {
			parts = append(parts, c.Text)
		}
	}
	return strings.Join(parts, "\n")
}

// CoreTool calls one of the daemon's core tools (tools_search, tools_call,
// apps_launch, apps_focus) through its /mcp endpoint, as an agent would.
func (r *RealDaemon) CoreTool(ctx context.Context, name string, args map[string]any) (ToolResult, error) {
	var out struct {
		Result struct {
			Content           []ToolContent  `json:"content"`
			StructuredContent map[string]any `json:"structuredContent"`
			IsError           bool           `json:"isError"`
		} `json:"result"`
		Error *struct {
			Message string `json:"message"`
		} `json:"error"`
	}
	err := r.rpc(ctx, "tools/call", map[string]any{"name": name, "arguments": args}, &out)
	if err != nil {
		return ToolResult{}, err
	}
	if out.Error != nil {
		return ToolResult{}, errors.New(out.Error.Message)
	}
	return ToolResult{out.Result.Content, out.Result.StructuredContent, out.Result.IsError}, nil
}

// CallTool runs an app's tool (tools_call "app.tool") through the gateway.
func (r *RealDaemon) CallTool(ctx context.Context, name string, args map[string]any) (ToolResult, error) {
	if args == nil {
		args = map[string]any{}
	}
	return r.CoreTool(ctx, "tools_call", map[string]any{"name": name, "args": args})
}

func (r *RealDaemon) rpc(ctx context.Context, method string, params any, out any) error {
	body, err := json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, r.URL+"/mcp", bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	req.Header.Set("Authorization", "Bearer "+r.token)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != 200 {
		return fmt.Errorf("daemon /mcp: status %d: %s", resp.StatusCode, data)
	}
	return json.Unmarshal(data, out)
}

// repoRoot finds the directory with go.work, starting at this file.
func repoRoot(t testing.TB) string {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("cannot locate the repo")
	}
	for dir := filepath.Dir(file); dir != filepath.Dir(dir); dir = filepath.Dir(dir) {
		if _, err := os.Stat(filepath.Join(dir, "go.work")); err == nil {
			return dir
		}
	}
	t.Fatal("no go.work above " + file)
	return ""
}

type lockedBuffer struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (l *lockedBuffer) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.b.Write(p)
}

func (l *lockedBuffer) String() string {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.b.String()
}
