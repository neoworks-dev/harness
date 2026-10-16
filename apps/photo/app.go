package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"sync"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
	"neoworks.dev/suite/sdk-go/tools"
)

// config is everything run needs from the outside.
type config struct {
	Socket   string
	Headless bool // no terminal UI: tools and presence only
	Open     string
	Launch   []string // how the daemon restarts the app (apps_launch)
	Term     terminal // stdin/stdout; nil forces headless
	Stderr   io.Writer
	Getenv   func(string) string
	// Ready, if set, is called once the app is registered (for tests).
	Ready func()
}

// app is the running photo editor: a daemon connection, the tool server and
// the editor state they share.
type app struct {
	cfg config
	c   *sdk.Client
	srv *tools.Server
	ed  *editor

	focused chan struct{} // cap 1: the agent moved
	redraw  chan struct{} // cap 1: something the UI shows changed

	noteMu sync.Mutex
	notes  []string // messages for the UI, oldest first
}

func poke(ch chan struct{}) {
	select {
	case ch <- struct{}{}:
	default:
	}
}

// logf reports something to the user: in the UI as a note, otherwise on stderr.
func (a *app) logf(format string, args ...any) {
	msg := fmt.Sprintf(format, args...)
	if a.ui() {
		a.noteMu.Lock()
		a.notes = append(a.notes, msg)
		a.noteMu.Unlock()
		poke(a.redraw)
		return
	}
	if a.cfg.Stderr != nil {
		fmt.Fprintf(a.cfg.Stderr, "photo-editor: %s\n", msg)
	}
}

func (a *app) takeNotes() []string {
	a.noteMu.Lock()
	defer a.noteMu.Unlock()
	n := a.notes
	a.notes = nil
	return n
}

// ui reports whether the terminal UI runs.
func (a *app) ui() bool { return !a.cfg.Headless && a.cfg.Term != nil && a.cfg.Term.interactive() }

// run starts the app and returns when ctx ends or the UI is quit.
func run(ctx context.Context, cfg config) error {
	a := &app{cfg: cfg, focused: make(chan struct{}, 1), redraw: make(chan struct{}, 1)}

	c, err := sdk.Dial(ctx, sdk.Options{
		Socket: cfg.Socket, Client: appName, Reconnect: true,
		Logf:    func(format string, args ...any) { a.logf(format, args...) },
		OnFocus: func(protocol.AgentFocus) { poke(a.focused); poke(a.redraw) },
	})
	if err != nil {
		if errors.Is(err, sdk.ErrDaemonDown) {
			return errors.New("the suite daemon is not running (start it with `bun run daemon`)")
		}
		return err
	}
	a.c = c
	defer c.Close()
	a.ed = newEditor(c)

	a.srv, err = tools.Start(tools.Options{
		Name: appName, Instructions: instructions, Tools: photoTools(a.ed),
		Logf: func(format string, args ...any) { a.logf(format, args...) },
	})
	if err != nil {
		return err
	}
	defer a.srv.Close()

	if cfg.Open != "" {
		if _, err := a.ed.openPath(ctx, cfg.Open); err != nil {
			a.logf("cannot open %s: %v", cfg.Open, err)
		}
	}
	if err := a.publish(ctx); err != nil {
		return fmt.Errorf("registering: %w", err)
	}
	a.logf("registered as %s; tools at %s", appName, a.srv.URL())
	defer func() {
		// Unregister explicitly: the connector then shows as closed at once.
		bye, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		p := presence(a.srv, a.cfg.Launch, nil)
		p.Open = false
		c.SetPresence(bye, p)
	}()

	pumpDone := make(chan struct{})
	pumpCtx, stopPump := context.WithCancel(ctx)
	go func() { defer close(pumpDone); a.pump(pumpCtx) }()
	defer func() { stopPump(); <-pumpDone }()

	if cfg.Ready != nil {
		cfg.Ready()
	}
	if a.ui() {
		return a.runUI(ctx)
	}
	select {
	case <-ctx.Done():
		return nil
	case <-c.Done():
		return c.Err()
	}
}

// publish sends the presence for the current image.
func (a *app) publish(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	d, _ := a.ed.current()
	return a.c.SetPresence(ctx, presence(a.srv, a.cfg.Launch, d))
}

// updateContext tells the daemon which image is open now.
func (a *app) updateContext(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	d, _ := a.ed.current()
	return a.c.SetContext(ctx, contextOf(d))
}

// pump keeps the presence context in step with the open image and wakes the
// UI when the image changes.
func (a *app) pump(ctx context.Context) {
	for {
		select {
		case <-ctx.Done():
			return
		case <-a.ed.changed:
			if err := a.updateContext(ctx); err != nil && ctx.Err() == nil {
				a.logf("updating context: %v", err)
			}
			poke(a.redraw)
		}
	}
}
