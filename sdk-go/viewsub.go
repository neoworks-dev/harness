package sdk

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"

	protocol "neoworks.dev/neo/protocol/go"
)

// ViewUpdate is one change to a followed view: either a patch, or (after a
// reconnect) a fresh snapshot that replaces everything.
type ViewUpdate struct {
	Snapshot *protocol.ViewSnapshot
	Patch    *protocol.ViewPatch
}

// ApplyTo applies the update to a state: Reset for a snapshot, Apply for a patch.
func (u ViewUpdate) ApplyTo(v *ViewState) error {
	switch {
	case u.Snapshot != nil:
		v.Reset(*u.Snapshot)
	case u.Patch != nil:
		return v.Apply(*u.Patch)
	}
	return nil
}

// ViewSubscription follows one session's resolved view: the snapshot taken
// when it started, then every change.
type ViewSubscription struct {
	// Snapshot is the view as of subscribing. Updates delivers what follows.
	Snapshot protocol.ViewSnapshot

	c       *Client
	session string
	wake    chan struct{} // cap 1: the queue has something new
	out     chan ViewUpdate
	stop    chan struct{} // closed by Close: abandon undelivered updates
	stopped sync.Once

	mu    sync.Mutex
	ready bool // the first snapshot arrived; patches are meaningful from here
	queue []ViewUpdate
	ended bool
	err   error
}

func newViewSubscription(c *Client, session string) *ViewSubscription {
	s := &ViewSubscription{
		c:       c,
		session: session,
		wake:    make(chan struct{}, 1),
		out:     make(chan ViewUpdate),
		stop:    make(chan struct{}),
	}
	go s.pump()
	return s
}

// State returns a ViewState holding the subscription's snapshot, to which
// the caller applies Updates as they arrive.
func (s *ViewSubscription) State() *ViewState {
	v := &ViewState{}
	v.Reset(s.Snapshot)
	return v
}

// Updates delivers changes in order, also across reconnects: after the
// connection came back the first update is a Snapshot, since patches missed
// while away cannot be replayed. It is closed when the subscription ends;
// Err says why. Updates are queued without limit, so a slow reader never
// blocks the connection.
func (s *ViewSubscription) Updates() <-chan ViewUpdate { return s.out }

// Err reports why Updates was closed: nil after Close, ErrClosed or
// ErrConnectionLost when the client went away, or an *Error if the daemon
// refused to resubscribe after a reconnect.
func (s *ViewSubscription) Err() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.err
}

// Close unsubscribes. Updates still queued are dropped.
func (s *ViewSubscription) Close() error {
	c := s.c
	c.mu.Lock()
	if c.views[s.session] == s {
		delete(c.views, s.session)
	}
	finished := c.finished
	c.mu.Unlock()

	s.stopped.Do(func() { close(s.stop) })
	s.finish(nil)
	if finished {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, err := request[protocol.BusAck](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.ViewUnsubscribe{ID: id, Payload: protocol.ViewRef{Session: s.session}}
	})
	return err
}

// snapshot is called by the reader goroutine for each view.snapshot. The
// first is the reply SubscribeView waits for; later ones follow a resubscribe.
func (s *ViewSubscription) snapshot(snap protocol.ViewSnapshot) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ended {
		return
	}
	if !s.ready {
		s.ready = true
		return
	}
	s.queue = append(s.queue, ViewUpdate{Snapshot: &snap})
	s.signal()
}

// patch is called by the reader goroutine for each view.patch.
func (s *ViewSubscription) patch(p protocol.ViewPatch) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ended || !s.ready {
		return
	}
	s.queue = append(s.queue, ViewUpdate{Patch: &p})
	s.signal()
}

func (s *ViewSubscription) isReady() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.ready
}

// finish ends the subscription once the queue is drained. The first call wins.
func (s *ViewSubscription) finish(err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ended {
		return
	}
	s.ended, s.err = true, err
	s.signal()
}

func (s *ViewSubscription) signal() {
	select {
	case s.wake <- struct{}{}:
	default:
	}
}

// pump moves queued updates to the Updates channel.
func (s *ViewSubscription) pump() {
	defer close(s.out)
	for {
		s.mu.Lock()
		batch, ended := s.queue, s.ended
		s.queue = nil
		s.mu.Unlock()
		for _, u := range batch {
			select {
			case s.out <- u:
			case <-s.stop:
				return
			}
		}
		if len(batch) > 0 {
			continue
		}
		if ended {
			return
		}
		select {
		case <-s.wake:
		case <-s.stop:
			return
		}
	}
}

// SubscribeView follows the resolved view of a session. It returns once the
// daemon has sent the snapshot. Subscribing to a session's view twice on one
// client is an error. With Options.Reconnect the subscription resubscribes
// after a reconnect and delivers the fresh snapshot as an update.
func (c *Client) SubscribeView(ctx context.Context, session string) (*ViewSubscription, error) {
	c.mu.Lock()
	if c.views[session] != nil {
		c.mu.Unlock()
		return nil, fmt.Errorf("already following the view of session %s", session)
	}
	sub := newViewSubscription(c, session)
	// Registered before the request goes out, like session subscriptions.
	c.views[session] = sub
	c.mu.Unlock()

	res, err := request[protocol.ViewSnapshotMessage](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.ViewSubscribe{ID: id, Payload: protocol.ViewSubscribeParams{Session: session}}
	})
	if err != nil {
		c.mu.Lock()
		if c.views[session] == sub {
			delete(c.views, session)
		}
		c.mu.Unlock()
		sub.stopped.Do(func() { close(sub.stop) })
		sub.finish(err)
		return nil, err
	}
	sub.Snapshot = res.Payload
	return sub, nil
}

// Action presses a button: it sends ui.action for the item ref in the
// session's view. The daemon runs app.tool actions as that app's tool, or
// hands the action to the agent when the app is not open; Action returns
// once it accepted the request. Actions starting with "copy:" are the
// client's own (they copy text) and are refused here.
func (c *Client) Action(ctx context.Context, session, ref, action string, args json.RawMessage) error {
	if strings.HasPrefix(action, "copy:") {
		return fmt.Errorf("%q is a client-side action; copy the text instead", action)
	}
	_, err := request[protocol.BusAck](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.UiActionMessage{ID: id, Payload: protocol.UiAction{
			Session: session, Ref: ref, Action: action, Args: args,
		}}
	})
	return err
}

// Theme returns the latest theme the daemon sent: after the handshake and
// whenever it changes. ok is false until the first one arrived.
func (c *Client) Theme() (theme protocol.Theme, ok bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.theme == nil {
		return protocol.Theme{}, false
	}
	return *c.theme, true
}

// WaitTheme returns the current theme, waiting for the first one if needed.
func (c *Client) WaitTheme(ctx context.Context) (protocol.Theme, error) {
	select {
	case <-c.themed:
	case <-c.done:
		return protocol.Theme{}, c.Err()
	case <-ctx.Done():
		return protocol.Theme{}, ctx.Err()
	}
	theme, _ := c.Theme()
	return theme, nil
}
