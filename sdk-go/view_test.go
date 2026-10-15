package sdk_test

import (
	"errors"
	"reflect"
	"testing"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
	"neoworks.dev/suite/sdk-go/sdktest"
)

func nextUpdate(t *testing.T, sub *sdk.ViewSubscription) sdk.ViewUpdate {
	t.Helper()
	select {
	case u, ok := <-sub.Updates():
		if !ok {
			t.Fatalf("updates closed: %v", sub.Err())
		}
		return u
	case <-time.After(5 * time.Second):
		t.Fatal("no view update")
		return sdk.ViewUpdate{}
	}
}

func TestSubscribeViewSnapshotThenPatches(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(),
		protocol.UserMessage{App: "cli", Text: "hi"},
		protocol.AgentMessageChunk{MessageID: "m1", Text: "Hello"},
	)
	c := dial(t, d, sdk.Options{})
	sub, err := c.SubscribeView(ctxT(t), "s1")
	if err != nil {
		t.Fatal(err)
	}
	if sub.Snapshot.Session != "s1" || sub.Snapshot.Seq != 2 || len(sub.Snapshot.Items) != 2 {
		t.Fatalf("snapshot = %+v", sub.Snapshot)
	}
	state := sub.State()
	if refs(state) != "u1,m-m1" {
		t.Fatalf("refs = %s", refs(state))
	}

	// A streamed second chunk arrives as a blocks op.
	d.Append("s1", protocol.AgentMessageChunk{MessageID: "m1", Text: " world\n\nSecond"})
	u := nextUpdate(t, sub)
	if u.Patch == nil || u.Snapshot != nil || u.Patch.Seq != 3 {
		t.Fatalf("update = %+v", u)
	}
	if _, ok := u.Patch.Ops[0].Value.(protocol.BlocksViewOp); !ok {
		t.Fatalf("op = %T", u.Patch.Ops[0].Value)
	}
	if err := u.ApplyTo(state); err != nil {
		t.Fatal(err)
	}
	m, _ := state.Item("m-m1")
	if got := blockTexts(t, m.Component); !reflect.DeepEqual(got, []string{"Hello world", "Second"}) {
		t.Errorf("streamed = %v", got)
	}

	// A new item and a manual patch with a nested op.
	d.Append("s1", protocol.ToolCall{ToolCallID: "t1", Title: "run", Status: protocol.ToolCallStatusInProgress})
	nextUpdate(t, sub).ApplyTo(state)
	if refs(state) != "u1,m-m1,c-t1" {
		t.Errorf("refs = %s", refs(state))
	}
	d.PatchView("s1", remove("u1"))
	nextUpdate(t, sub).ApplyTo(state)
	if refs(state) != "m-m1,c-t1" || state.Seq != 4 {
		t.Errorf("refs %s seq %d", refs(state), state.Seq)
	}
	// The daemon and the client agree.
	if got, want := state.Items(), d.ViewItems("s1"); !reflect.DeepEqual(got, want) {
		t.Errorf("client %+v\ndaemon %+v", got, want)
	}

	if err := sub.Close(); err != nil {
		t.Fatal(err)
	}
	if _, open := <-sub.Updates(); open {
		t.Error("updates still open after Close")
	}
	// Subscribing again after Close works and yields a fresh snapshot.
	sub2, err := c.SubscribeView(ctxT(t), "s1")
	if err != nil || len(sub2.Snapshot.Items) != 2 {
		t.Errorf("resubscribe: %v %+v", err, sub2)
	}
}

func TestSubscribeViewErrors(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)

	var apiErr *sdk.Error
	if _, err := c.SubscribeView(ctx, "nope"); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeNotFound {
		t.Errorf("unknown session: %v", err)
	}
	if _, err := c.SubscribeView(ctx, "s1"); err != nil {
		t.Fatal(err)
	}
	if _, err := c.SubscribeView(ctx, "s1"); err == nil {
		t.Error("second subscription to one view succeeded")
	}
}

func TestViewAndEventSubscriptionsAreIndependent(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(), protocol.UserMessage{App: "cli", Text: "hi"})
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)
	events, err := c.Subscribe(ctx, "s1", 1)
	if err != nil {
		t.Fatal(err)
	}
	view, err := c.SubscribeView(ctx, "s1")
	if err != nil {
		t.Fatal(err)
	}
	d.Append("s1", protocol.UserMessage{App: "cli", Text: "again"})
	if rec := <-events.Events(); rec.Seq != 2 {
		t.Errorf("event seq %d", rec.Seq)
	}
	if u := nextUpdate(t, view); u.Patch == nil || u.Patch.Seq != 2 {
		t.Errorf("update %+v", u)
	}
	// Closing one leaves the other running.
	view.Close()
	d.Append("s1", protocol.UserMessage{App: "cli", Text: "third"})
	if rec := <-events.Events(); rec.Seq != 3 {
		t.Errorf("event seq %d", rec.Seq)
	}
}

func TestViewReconnectDeliversFreshSnapshot(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(), protocol.UserMessage{App: "cli", Text: "one"})
	c := dial(t, d, sdk.Options{Reconnect: true, Logf: t.Logf})
	sub, err := c.SubscribeView(ctxT(t), "s1")
	if err != nil {
		t.Fatal(err)
	}
	state := sub.State()

	d.Stop()
	d.Append("s1", protocol.UserMessage{App: "cli", Text: "two"}) // missed while away
	d.Append("s1", protocol.UserMessage{App: "cli", Text: "three"})
	d.Start()

	u := nextUpdate(t, sub)
	if u.Snapshot == nil || u.Patch != nil || u.Snapshot.Seq != 3 {
		t.Fatalf("first update after reconnect = %+v", u)
	}
	if err := u.ApplyTo(state); err != nil || refs(state) != "u1,u2,u3" {
		t.Fatalf("after snapshot: %v, refs %s", err, refs(state))
	}
	// And patches flow again on the new connection.
	d.Append("s1", protocol.UserMessage{App: "cli", Text: "four"})
	u = nextUpdate(t, sub)
	if u.Patch == nil || u.Patch.Seq != 4 {
		t.Fatalf("live update = %+v", u)
	}
	u.ApplyTo(state)
	if refs(state) != "u1,u2,u3,u4" {
		t.Errorf("refs = %s", refs(state))
	}
	if n := len(d.ViewSubscribes()); n != 2 {
		t.Errorf("view.subscribe sent %d times, want 2", n)
	}
}

func TestViewEndsWithClient(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	c := dial(t, d, sdk.Options{})
	sub, err := c.SubscribeView(ctxT(t), "s1")
	if err != nil {
		t.Fatal(err)
	}
	d.DropConnections()
	select {
	case _, open := <-sub.Updates():
		if open {
			t.Fatal("unexpected update")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("updates not closed")
	}
	if !errors.Is(sub.Err(), sdk.ErrConnectionLost) {
		t.Errorf("Err = %v", sub.Err())
	}
}

func TestTheme(t *testing.T) {
	d := sdktest.New(t)
	got := make(chan protocol.Theme, 4)
	c := dial(t, d, sdk.Options{OnTheme: func(th protocol.Theme) { got <- th }})
	th, err := c.WaitTheme(ctxT(t))
	if err != nil || th.Name != "default-dark" || th.Colors.Accent != "#7aa2f7" {
		t.Fatalf("WaitTheme = %+v, %v", th, err)
	}
	if cur, ok := c.Theme(); !ok || cur != th {
		t.Errorf("Theme = %+v, %v", cur, ok)
	}
	if first := <-got; first != th {
		t.Errorf("OnTheme first = %+v", first)
	}

	next := sdktest.DefaultTheme()
	next.Name, next.Colors.Accent = "light", "#0000ff"
	d.SetTheme(next)
	if changed := <-got; changed != next {
		t.Errorf("OnTheme after change = %+v", changed)
	}
	if cur, _ := c.Theme(); cur != next {
		t.Errorf("Theme after change = %+v", cur)
	}
}

func TestAction(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	c := dial(t, d, sdk.Options{})
	ctx := ctxT(t)

	args := []byte(`{"asset":"asset://img-7f3"}`)
	if err := c.Action(ctx, "s1", "c-t1", "photo-editor.open_layer", args); err != nil {
		t.Fatal(err)
	}
	if err := c.Action(ctx, "s1", "c-t1", "photo-editor.undo", nil); err != nil {
		t.Fatal(err)
	}
	got := d.Actions()
	if len(got) != 2 || got[0].Session != "s1" || got[0].Ref != "c-t1" || got[0].Action != "photo-editor.open_layer" || string(got[0].Args) != string(args) {
		t.Fatalf("actions = %+v", got)
	}
	if got[1].Args != nil {
		t.Errorf("args = %s, want none", got[1].Args)
	}

	var apiErr *sdk.Error
	if err := c.Action(ctx, "nope", "r", "a.b", nil); !errors.As(err, &apiErr) || apiErr.Code != protocol.ErrorCodeNotFound {
		t.Errorf("unknown session: %v", err)
	}
	// copy: is the client's job and never goes out.
	if err := c.Action(ctx, "s1", "r", "copy:hello", nil); err == nil {
		t.Error("copy: action was sent")
	}
	if n := len(d.Actions()); n != 2 {
		t.Errorf("%d actions recorded", n)
	}
}

func TestSubscribeLiveSkipsReplay(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(),
		protocol.UserMessage{App: "a", Text: "1"}, protocol.UserMessage{App: "a", Text: "2"})
	c := dial(t, d, sdk.Options{Reconnect: true})
	sub, err := c.SubscribeLive(ctxT(t), "s1")
	if err != nil {
		t.Fatal(err)
	}
	if len(sub.Replay) != 0 || sub.Info.LastSeq != 2 {
		t.Fatalf("replay %d, info %+v", len(sub.Replay), sub.Info)
	}
	d.Append("s1", protocol.UserMessage{App: "a", Text: "3"})
	if rec := <-sub.Events(); rec.Seq != 3 {
		t.Fatalf("seq %d", rec.Seq)
	}

	// After a reconnect it resumes after what it saw, not from the start.
	d.DropConnections()
	sdktest.Eventually(t, "resubscribe", func() bool { return len(d.Subscribes()) == 2 })
	if from := d.Subscribes()[1].FromSeq; from == nil || *from != 4 {
		t.Errorf("resubscribed from %v, want 4", from)
	}
	d.Append("s1", protocol.UserMessage{App: "a", Text: "4"})
	if rec := <-sub.Events(); rec.Seq != 4 {
		t.Fatalf("seq %d", rec.Seq)
	}
}
