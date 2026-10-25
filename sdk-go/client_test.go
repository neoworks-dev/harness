package sdk_test

import (
	"context"
	"errors"
	"net"
	"path/filepath"
	"slices"
	"sync"
	"testing"
	"time"

	sdk "neoworks.dev/neo/sdk-go"
	"neoworks.dev/neo/sdk-go/sdktest"

	protocol "neoworks.dev/neo/protocol/go"
)

func ptr[T any](v T) *T { return &v }

func ctxT(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	return ctx
}

func dial(t *testing.T, d *sdktest.Daemon, opts sdk.Options) *sdk.Client {
	t.Helper()
	opts.Socket = d.Socket
	c, err := sdk.Dial(ctxT(t), opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

func TestSocketPath(t *testing.T) {
	env := func(m map[string]string) func(string) string { return func(k string) string { return m[k] } }
	tests := []struct {
		name string
		flag string
		env  map[string]string
		want string
	}{
		{"override wins", "/f.sock", map[string]string{"NEO_SOCKET": "/e.sock", "XDG_RUNTIME_DIR": "/run/user/1"}, "/f.sock"},
		{"env over runtime dir", "", map[string]string{"NEO_SOCKET": "/e.sock", "XDG_RUNTIME_DIR": "/run/user/1"}, "/e.sock"},
		{"runtime dir", "", map[string]string{"XDG_RUNTIME_DIR": "/run/user/1"}, "/run/user/1/neo/bus.sock"},
		{"tmp fallback", "", nil, "/tmp/neo-1000/bus.sock"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := sdk.SocketPath(tc.flag, env(tc.env), 1000); got != tc.want {
				t.Errorf("got %q, want %q", got, tc.want)
			}
		})
	}
}

func TestDialHandshake(t *testing.T) {
	d := sdktest.New(t)
	c := dial(t, d, sdk.Options{Client: "tester"})
	if w := c.Welcome(); w.PID != 4242 || w.DaemonVersion != "test" {
		t.Errorf("welcome = %+v", w)
	}
	if h := d.Hellos(); len(h) != 1 || h[0] != (protocol.Hello{Protocol: 1, Client: "tester"}) {
		t.Errorf("hellos = %+v", h)
	}
}

func TestDialDaemonDown(t *testing.T) {
	dir := t.TempDir()
	_, err := sdk.Dial(ctxT(t), sdk.Options{Socket: filepath.Join(dir, "missing.sock")})
	if !errors.Is(err, sdk.ErrDaemonDown) {
		t.Errorf("err = %v, want ErrDaemonDown", err)
	}
	// A socket file nobody listens on refuses the connection.
	stale := filepath.Join(dir, "stale.sock")
	l, err := net.Listen("unix", stale)
	if err != nil {
		t.Fatal(err)
	}
	l.(*net.UnixListener).SetUnlinkOnClose(false)
	l.Close()
	if _, err := sdk.Dial(ctxT(t), sdk.Options{Socket: stale}); !errors.Is(err, sdk.ErrDaemonDown) {
		t.Errorf("stale socket: err = %v, want ErrDaemonDown", err)
	}
}

func TestSessionRequests(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "one", time.Now())
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)

	sessions, err := c.ListSessions(ctx)
	if err != nil || len(sessions) != 1 || sessions[0].ID != "s1" {
		t.Fatalf("ListSessions = %+v, %v", sessions, err)
	}
	info, err := c.CreateSession(ctx, protocol.SessionCreateParams{Cwd: ptr("/proj"), Title: ptr("T")})
	if err != nil || info.ID == "" || info.Cwd != "/proj" {
		t.Fatalf("CreateSession = %+v, %v", info, err)
	}
	in := protocol.SessionInput{Session: "s1", App: "tester", Text: "hi", Images: []string{"/tmp/a.png"}}
	if err := c.Input(ctx, in); err != nil {
		t.Fatal(err)
	}
	if got := d.Inputs(); len(got) != 1 || !slices.Equal(got[0].Images, in.Images) || got[0].Text != "hi" {
		t.Errorf("inputs = %+v", got)
	}
	if err := c.Cancel(ctx, "s1"); err != nil {
		t.Fatal(err)
	}
	if got := d.Cancels(); !slices.Equal(got, []string{"s1"}) {
		t.Errorf("cancels = %v", got)
	}

	var apiErr *sdk.Error
	if err := c.Input(ctx, protocol.SessionInput{Session: "nope", App: "x", Text: "x"}); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeNotFound {
		t.Errorf("input to missing session: err = %v", err)
	}
}

func TestSubscribeReplayThenLive(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(),
		protocol.UserMessage{App: "a", Text: "one"},
		protocol.AgentMessageChunk{MessageID: "m", Text: "two"},
		protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
	)
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)

	sub, err := c.Subscribe(ctx, "s1", 2)
	if err != nil {
		t.Fatal(err)
	}
	if len(sub.Replay) != 2 || sub.Replay[0].Seq != 2 || sub.Replay[1].Seq != 3 || sub.Info.LastSeq != 3 {
		t.Fatalf("replay = %+v, info = %+v", sub.Replay, sub.Info)
	}
	d.Append("s1", protocol.UserMessage{App: "b", Text: "live", Images: []string{"/x.png"}})
	rec := <-sub.Events()
	um, ok := rec.Event.Value.(protocol.UserMessage)
	if rec.Seq != 4 || !ok || um.Text != "live" || len(um.Images) != 1 {
		t.Errorf("live event = %+v", rec)
	}

	if _, err := c.Subscribe(ctx, "s1", 1); err == nil {
		t.Error("second subscription to one session succeeded")
	}
	var apiErr *sdk.Error
	if _, err := c.Subscribe(ctx, "nope", 1); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeNotFound {
		t.Errorf("missing session: err = %v", err)
	}

	if err := sub.Close(); err != nil {
		t.Fatal(err)
	}
	if _, open := <-sub.Events(); open {
		t.Error("Events still open after Close")
	}
	if sub.Err() != nil {
		t.Errorf("Err after Close = %v", sub.Err())
	}
	// Closed means closed: a new subscription is possible again.
	if sub, err = c.Subscribe(ctx, "s1", 4); err != nil || len(sub.Replay) != 1 {
		t.Errorf("resubscribe: %v, %+v", err, sub)
	}
}

// A consumer that is busy making requests must not stall event delivery or
// the replies to its own requests.
func TestSlowSubscriberDoesNotBlockRequests(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)
	sub, err := c.Subscribe(ctx, "s1", 1)
	if err != nil {
		t.Fatal(err)
	}
	const n = 200
	for range n {
		d.Append("s1", protocol.AgentMessageChunk{MessageID: "m", Text: "x"})
	}
	if _, err := c.ListSessions(ctx); err != nil {
		t.Fatal(err)
	}
	for want := int64(1); want <= n; want++ {
		if rec := <-sub.Events(); rec.Seq != want {
			t.Fatalf("got seq %d, want %d", rec.Seq, want)
		}
	}
}

func TestConnectorsAndPresence(t *testing.T) {
	d := sdktest.New(t)
	d.SetConnector(protocol.ConnectorInfo{
		App: "photo", Kind: protocol.ConnectorKindApp, Tools: []protocol.ConnectorTool{{Name: "photo.crop", Description: "Crop"}},
	})
	c := dial(t, d, sdk.Options{Client: "photo"})
	ctx := ctxT(t)

	list, err := c.ListConnectors(ctx)
	if err != nil || len(list) != 1 || list[0].Connected {
		t.Fatalf("ListConnectors = %+v, %v", list, err)
	}

	p := protocol.AppPresence{
		App: "photo", Kind: protocol.ConnectorKindApp, Title: ptr("Photo editor"), Open: true,
		Context: map[string]string{"file": "a.png"},
		Mcp:     &protocol.McpEndpoint{URL: "http://127.0.0.1:9/mcp", Headers: map[string]string{"x": "y"}},
		Launch:  &protocol.LaunchSpec{Command: []string{"photo"}},
	}
	if err := c.SetPresence(ctx, p); err != nil {
		t.Fatal(err)
	}
	got, ok := d.Presence("photo")
	if !ok || got.Mcp == nil || got.Mcp.URL != p.Mcp.URL || got.Context["file"] != "a.png" || got.Launch.Command[0] != "photo" {
		t.Errorf("registered presence = %+v", got)
	}
	list, _ = c.ListConnectors(ctx)
	if !list[0].Connected || list[0].V != 1 || len(list[0].Tools) != 1 {
		t.Errorf("after presence: %+v", list[0])
	}

	if err := c.SetPresence(ctx, protocol.AppPresence{App: "photo", Kind: protocol.ConnectorKindApp, Open: false}); err != nil {
		t.Fatal(err)
	}
	if _, open := d.Presence("photo"); open {
		t.Error("presence open:false left the connector registered")
	}

	var apiErr *sdk.Error
	if err := c.SetPresence(ctx, protocol.AppPresence{Kind: protocol.ConnectorKindApp, Open: true}); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeBadRequest {
		t.Errorf("invalid presence: err = %v", err)
	}
}

func TestClosingConnectionUnregisters(t *testing.T) {
	d := sdktest.New(t)
	c := dial(t, d, sdk.Options{})
	if err := c.SetPresence(ctxT(t), protocol.AppPresence{App: "a", Kind: protocol.ConnectorKindApp, Open: true}); err != nil {
		t.Fatal(err)
	}
	c.Close()
	sdktest.Eventually(t, "unregistration", func() bool { _, open := d.Presence("a"); return !open })
}

func TestConnectionLostWithoutReconnect(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	c := dial(t, d, sdk.Options{})
	sub, err := c.Subscribe(ctxT(t), "s1", 1)
	if err != nil {
		t.Fatal(err)
	}
	d.DropConnections()

	select {
	case <-c.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("Done not closed")
	}
	if !errors.Is(c.Err(), sdk.ErrConnectionLost) {
		t.Errorf("Err = %v", c.Err())
	}
	if _, open := <-sub.Events(); open {
		t.Error("Events still open")
	}
	if !errors.Is(sub.Err(), sdk.ErrConnectionLost) {
		t.Errorf("sub.Err = %v", sub.Err())
	}
	if _, err := c.ListSessions(ctxT(t)); !errors.Is(err, sdk.ErrConnectionLost) {
		t.Errorf("request after loss: %v", err)
	}
	c.Close() // idempotent
}

func TestCloseEndsClient(t *testing.T) {
	d := sdktest.New(t)
	c := dial(t, d, sdk.Options{})
	c.Close()
	if !errors.Is(c.Err(), sdk.ErrClosed) {
		t.Errorf("Err = %v", c.Err())
	}
	if _, err := c.ListSessions(ctxT(t)); !errors.Is(err, sdk.ErrClosed) {
		t.Errorf("request after Close: %v", err)
	}
}

func TestReconnectRestoresPresenceAndSubscriptions(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(),
		protocol.UserMessage{App: "a", Text: "1"},
		protocol.UserMessage{App: "a", Text: "2"},
	)
	c := dial(t, d, sdk.Options{Client: "app", Reconnect: true, Logf: t.Logf})
	ctx := ctxT(t)
	p := protocol.AppPresence{App: "app", Kind: protocol.ConnectorKindApp, Open: true}
	if err := c.SetPresence(ctx, p); err != nil {
		t.Fatal(err)
	}
	sub, err := c.Subscribe(ctx, "s1", 1)
	if err != nil || len(sub.Replay) != 2 {
		t.Fatalf("subscribe: %v, %+v", err, sub)
	}
	d.Append("s1", protocol.UserMessage{App: "a", Text: "3"})
	if rec := <-sub.Events(); rec.Seq != 3 {
		t.Fatalf("seq %d", rec.Seq)
	}

	// Daemon goes away; events happen while the client is disconnected.
	d.Stop()
	sdktest.Eventually(t, "presence dropped", func() bool { _, open := d.Presence("app"); return !open })
	d.Append("s1", protocol.UserMessage{App: "a", Text: "4"})
	d.Append("s1", protocol.UserMessage{App: "a", Text: "5"})

	// A request issued while down waits for the new connection.
	listed := make(chan error, 1)
	go func() { _, err := c.ListSessions(ctx); listed <- err }()
	time.Sleep(100 * time.Millisecond)
	d.Start()

	if err := <-listed; err != nil {
		t.Errorf("request across reconnect: %v", err)
	}
	for _, want := range []int64{4, 5} {
		if rec := <-sub.Events(); rec.Seq != want {
			t.Fatalf("after reconnect got seq %d, want %d", rec.Seq, want)
		}
	}
	d.Append("s1", protocol.UserMessage{App: "a", Text: "6"})
	if rec := <-sub.Events(); rec.Seq != 6 {
		t.Fatalf("live after reconnect: seq %d", rec.Seq)
	}

	sdktest.Eventually(t, "presence restored", func() bool { _, open := d.Presence("app"); return open })
	if h := d.Hellos(); len(h) != 2 || h[1].Client != "app" {
		t.Errorf("hellos = %+v", h)
	}
	subs := d.Subscribes()
	if last := subs[len(subs)-1]; last.FromSeq == nil || *last.FromSeq != 4 {
		t.Errorf("resubscribe params = %+v, want fromSeq 4", subs)
	}
	select {
	case rec := <-sub.Events():
		t.Errorf("duplicate event %+v", rec)
	case <-time.After(100 * time.Millisecond):
	}
}

func TestReconnectEndsSubscriptionOfVanishedSession(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	c := dial(t, d, sdk.Options{Reconnect: true})
	sub, err := c.Subscribe(ctxT(t), "s1", 1)
	if err != nil {
		t.Fatal(err)
	}
	d.Stop()
	d.Forget("s1")
	d.Start()
	select {
	case _, open := <-sub.Events():
		if open {
			t.Fatal("unexpected event")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("subscription not ended")
	}
	var apiErr *sdk.Error
	if !errors.As(sub.Err(), &apiErr) {
		t.Errorf("sub.Err = %v", sub.Err())
	}
}

// register opens a connector from its own client.
func register(t *testing.T, d *sdktest.Daemon, app string, home bool, pid int64) *sdk.Client {
	t.Helper()
	c := dial(t, d, sdk.Options{Client: app})
	p := protocol.AppPresence{App: app, Kind: protocol.ConnectorKindApp, Open: true, PID: &pid}
	if home {
		p.Home = ptr(true)
	}
	if err := c.SetPresence(ctxT(t), p); err != nil {
		t.Fatal(err)
	}
	return c
}

func TestFocus(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	register(t, d, "home", true, 100)
	register(t, d, "editor", false, 200)

	var got []protocol.AgentFocus
	var mu sync.Mutex
	c := dial(t, d, sdk.Options{OnFocus: func(f protocol.AgentFocus) {
		mu.Lock()
		defer mu.Unlock()
		got = append(got, f)
	}})
	ctx := ctxT(t)
	f, err := c.WaitFocus(ctx)
	if err != nil || f.Session != "s1" || f.Reason != protocol.FocusReasonInitial || f.App == nil || *f.App != "home" {
		t.Fatalf("initial focus = %+v, %v", f, err)
	}
	if cur, ok := c.Focus(); !ok || cur.Session != "s1" {
		t.Errorf("Focus = %+v, %v", cur, ok)
	}

	if err := c.MoveAgent(ctx, protocol.AgentMove{App: ptr("editor")}); err != nil {
		t.Fatal(err)
	}
	// The broadcast precedes the ack, so the latest focus is already there.
	if cur, _ := c.Focus(); cur.App == nil || *cur.App != "editor" || *cur.From != "home" || cur.Reason != protocol.FocusReasonUser {
		t.Errorf("after move to editor: %+v", cur)
	}
	if err := c.MoveAgent(ctx, protocol.AgentMove{}); err != nil {
		t.Fatal(err)
	}
	if cur, _ := c.Focus(); *cur.App != "home" {
		t.Errorf("after move home: %+v", cur)
	}
	sdktest.Eventually(t, "OnFocus calls", func() bool { mu.Lock(); defer mu.Unlock(); return len(got) == 3 })
	mu.Lock()
	if *got[1].App != "editor" || *got[2].App != "home" {
		t.Errorf("callbacks = %+v", got)
	}
	mu.Unlock()

	var apiErr *sdk.Error
	if err := c.MoveAgent(ctx, protocol.AgentMove{App: ptr("nope")}); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeNotFound {
		t.Errorf("move to missing app: %v", err)
	}
	if err := c.MoveAgent(ctx, protocol.AgentMove{Session: ptr("nope")}); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeNotFound {
		t.Errorf("move to missing session: %v", err)
	}
}

func TestFocusFollowsClosingAndSession(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now().Add(-time.Hour))
	d.AddSession("s2", "", time.Now())
	register(t, d, "home", true, 100)
	editor := register(t, d, "editor", false, 200)
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)
	if f, _ := c.WaitFocus(ctx); f.Session != "s2" {
		t.Fatalf("current session = %q, want the most recent", f.Session)
	}

	if err := c.MoveAgent(ctx, protocol.AgentMove{App: ptr("editor"), Session: ptr("s1")}); err != nil {
		t.Fatal(err)
	}
	if f, _ := c.Focus(); f.Session != "s1" || *f.App != "editor" || f.Reason != protocol.FocusReasonSession {
		t.Errorf("after session move: %+v", f)
	}
	editor.Close()
	sdktest.Eventually(t, "fall back home", func() bool {
		f, _ := c.Focus()
		return f.App != nil && *f.App == "home" && f.Reason == protocol.FocusReasonClosed
	})
}

func TestConnectorsNotifications(t *testing.T) {
	d := sdktest.New(t)
	register(t, d, "home", true, 100)

	lists := make(chan []protocol.ConnectorInfo, 16)
	c := dial(t, d, sdk.Options{OnConnectors: func(l []protocol.ConnectorInfo) { lists <- l }})
	sdktest.Eventually(t, "the initial list", func() bool { return len(c.Connectors()) == 1 })
	if h := c.Connectors()[0]; h.Home == nil || !*h.Home || h.PID == nil || *h.PID != 100 {
		t.Errorf("home connector = %+v", h)
	}

	editor := register(t, d, "editor", false, 200)
	sdktest.Eventually(t, "editor listed", func() bool {
		for _, i := range c.Connectors() {
			if i.App == "editor" && i.Connected {
				return true
			}
		}
		return false
	})
	editor.Close()
	sdktest.Eventually(t, "editor closed", func() bool {
		for _, i := range c.Connectors() {
			if i.App == "editor" && !i.Connected {
				return true
			}
		}
		return false
	})
	if len(lists) == 0 {
		t.Error("OnConnectors was never called")
	}
}

func TestFocusAfterReconnect(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	calls := make(chan protocol.AgentFocus, 8)
	c := dial(t, d, sdk.Options{Reconnect: true, OnFocus: func(f protocol.AgentFocus) { calls <- f }})
	<-calls
	d.MoveAgentTo("", protocol.FocusReasonAgent) // no-op: it is nowhere already
	d.DropConnections()
	select {
	case f := <-calls:
		if f.Reason != protocol.FocusReasonInitial {
			t.Errorf("focus after reconnect = %+v", f)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("no agent.focus after reconnect")
	}
	if _, ok := c.Focus(); !ok {
		t.Error("Focus lost")
	}
}

func TestSetContext(t *testing.T) {
	d := sdktest.New(t)
	bare := dial(t, d, sdk.Options{Client: "bare"})
	if err := bare.SetContext(ctxT(t), map[string]string{"a": "b"}); err == nil {
		t.Error("SetContext without a presence succeeded")
	}
	c := dial(t, d, sdk.Options{Client: "code-editor"})
	if err := c.SetPresence(ctxT(t), protocol.AppPresence{App: "code-editor", Kind: protocol.ConnectorKindApp, Open: true}); err != nil {
		t.Fatal(err)
	}
	want := map[string]string{"file": "/site/a.ts", "line": "40"}
	if err := c.SetContext(ctxT(t), want); err != nil {
		t.Fatal(err)
	}
	p, _ := d.Presence("code-editor")
	if p.Context["line"] != "40" || p.Context["file"] != "/site/a.ts" {
		t.Errorf("context %v", p.Context)
	}
}
