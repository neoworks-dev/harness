package sdk

import (
	"context"
	"encoding/json"
	"errors"
	"io/fs"
	"net"
	"net/http"
	"slices"
	"syscall"
	"time"

	"github.com/coder/websocket"
	protocol "neoworks.dev/suite/protocol/go"
)

const (
	// ProtocolVersion is the bus protocol version this SDK speaks.
	ProtocolVersion = 1

	// Events carry tool output and can be large.
	readLimit    = 16 << 20
	dialTimeout  = 5 * time.Second
	writeTimeout = 10 * time.Second
	minBackoff   = 50 * time.Millisecond
	maxBackoff   = 5 * time.Second
)

// handshake connects to the Unix socket, upgrades to WebSocket at /bus and
// exchanges hello/welcome.
func handshake(ctx context.Context, socket, client string) (*websocket.Conn, protocol.Welcome, error) {
	ctx, cancel := context.WithTimeout(ctx, dialTimeout)
	defer cancel()

	httpClient := &http.Client{Transport: &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "unix", socket)
		},
	}}
	// The host is ignored by the dialer above; it only has to be well-formed.
	ws, _, err := websocket.Dial(ctx, "ws://suite/bus", &websocket.DialOptions{HTTPClient: httpClient})
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) || errors.Is(err, syscall.ECONNREFUSED) {
			err = ErrDaemonDown
		}
		return nil, protocol.Welcome{}, err
	}
	ws.SetReadLimit(readLimit)

	fail := func(err error) (*websocket.Conn, protocol.Welcome, error) {
		ws.CloseNow()
		return nil, protocol.Welcome{}, err
	}
	hello := protocol.BusHello{Payload: protocol.Hello{Protocol: ProtocolVersion, Client: client}}
	data, err := json.Marshal(protocol.Envelope{Value: hello})
	if err != nil {
		return fail(err)
	}
	if err := ws.Write(ctx, websocket.MessageText, data); err != nil {
		return fail(err)
	}
	for {
		_, data, err := ws.Read(ctx)
		if err != nil {
			return fail(err)
		}
		var env protocol.Envelope
		if json.Unmarshal(data, &env) != nil {
			continue
		}
		switch v := env.Value.(type) {
		case protocol.BusWelcome:
			return ws, v.Payload, nil
		case protocol.BusError:
			return fail(&Error{v.Payload})
		}
	}
}

func (c *Client) write(ws *websocket.Conn, v protocol.EnvelopeVariant) error {
	data, err := json.Marshal(protocol.Envelope{Value: v})
	if err != nil {
		return err
	}
	// Not the caller's ctx: cancelling a write mid-frame closes the socket.
	ctx, cancel := context.WithTimeout(c.ctx, writeTimeout)
	defer cancel()
	return ws.Write(ctx, websocket.MessageText, data)
}

// supervise owns the connection: it reads frames until the connection ends
// and, with Options.Reconnect, replaces it.
func (c *Client) supervise(ws *websocket.Conn) {
	for {
		err := c.connLost(c.readLoop(ws))
		if !c.opts.Reconnect {
			c.shutdown(err)
			return
		}
		if ws = c.redial(); ws == nil {
			c.shutdown(err)
			return
		}
	}
}

func (c *Client) readLoop(ws *websocket.Conn) error {
	for {
		_, data, err := ws.Read(c.ctx)
		if err != nil {
			ws.CloseNow()
			return err
		}
		var env protocol.Envelope
		if json.Unmarshal(data, &env) != nil || env.Value == nil {
			continue // not a frame we can use
		}
		c.dispatch(env.Value)
	}
}

// replyID returns the request id a daemon reply answers.
func replyID(v protocol.EnvelopeVariant) (string, bool) {
	switch m := v.(type) {
	case protocol.BusAck:
		return m.ID, true
	case protocol.BusError:
		if m.ID != nil {
			return *m.ID, true
		}
	case protocol.SessionInfoMessage:
		return m.ID, true
	case protocol.SessionListResult:
		return m.ID, true
	case protocol.ViewSnapshotMessage:
		return m.ID, true
	case protocol.SessionSubscribed:
		return m.ID, true
	case protocol.ConnectorsListResult:
		return m.ID, true
	case protocol.AssetInfoMessage:
		return m.ID, true
	}
	return "", false
}

// dispatch routes one frame, in wire order, on the reader goroutine.
func (c *Client) dispatch(v protocol.EnvelopeVariant) {
	switch m := v.(type) {
	case protocol.SessionEvents:
		c.mu.Lock()
		sub := c.subs[m.Payload.Session]
		c.mu.Unlock()
		if sub != nil {
			sub.push(m.Payload)
		}
		return
	case protocol.ViewPatchMessage:
		c.mu.Lock()
		sub := c.views[m.Payload.Session]
		c.mu.Unlock()
		if sub != nil {
			sub.patch(m.Payload)
		}
		return
	case protocol.ThemeMessage:
		c.setTheme(m.Payload)
		return
	case protocol.AgentFocusMessage:
		c.setFocus(m.Payload)
		return
	case protocol.ConnectorsChanged:
		c.setConnectors(m.Payload.Connectors)
		return
	case protocol.FileChangedMessage:
		if c.opts.OnFileChanged != nil {
			c.notes.push(func() { c.opts.OnFileChanged(m.Payload) })
		}
		return
	case protocol.ConnectorsListResult:
		// A fresh snapshot, in wire order with the broadcasts: the latest wins.
		c.setConnectors(m.Payload.Connectors)
	case protocol.ViewSnapshotMessage:
		// In wire order with the patches that follow it; the first one is
		// also the reply SubscribeView waits for (routed below).
		c.mu.Lock()
		sub := c.views[m.Payload.Session]
		c.mu.Unlock()
		if sub != nil {
			sub.snapshot(m.Payload)
		}
	case protocol.SessionSubscribed:
		// Flip to live before the waiter wakes, so the next frame is not
		// mistaken for replay.
		c.mu.Lock()
		sub := c.subs[m.Payload.Session.ID]
		c.mu.Unlock()
		if sub != nil {
			sub.markReady()
		}
	}
	id, ok := replyID(v)
	if !ok {
		if e, isErr := v.(protocol.BusError); isErr {
			c.logf("daemon error: %s", e.Payload.Message)
		}
		return
	}
	c.mu.Lock()
	ch := c.pending[id]
	delete(c.pending, id)
	c.mu.Unlock()
	if ch == nil {
		return
	}
	if e, isErr := v.(protocol.BusError); isErr {
		ch <- reply{err: &Error{e.Payload}}
		return
	}
	ch <- reply{v: v}
}

func (c *Client) setFocus(f protocol.AgentFocus) {
	c.mu.Lock()
	first := c.focus == nil
	c.focus = &f
	c.mu.Unlock()
	if first {
		close(c.focused)
	}
	if c.opts.OnFocus != nil {
		c.notes.push(func() { c.opts.OnFocus(f) })
	}
}

func (c *Client) setTheme(t protocol.Theme) {
	c.mu.Lock()
	first := c.theme == nil
	c.theme = &t
	c.mu.Unlock()
	if first {
		close(c.themed)
	}
	if c.opts.OnTheme != nil {
		c.notes.push(func() { c.opts.OnTheme(t) })
	}
}

func (c *Client) setConnectors(list []protocol.ConnectorInfo) {
	c.mu.Lock()
	c.conns = list
	c.mu.Unlock()
	if c.opts.OnConnectors != nil {
		c.notes.push(func() { c.opts.OnConnectors(slices.Clone(list)) })
	}
}

// connLost marks the connection gone and fails the requests waiting on it.
func (c *Client) connLost(cause error) error {
	c.mu.Lock()
	if c.ws != nil {
		c.up = make(chan struct{})
		c.ws = nil
	}
	pending := c.pending
	c.pending = map[string]chan reply{}
	c.mu.Unlock()

	err := c.lostError(cause)
	for _, ch := range pending {
		ch <- reply{err: err}
	}
	return err
}

// redial reconnects with backoff and restores state. It returns nil once the
// client is closed.
func (c *Client) redial() *websocket.Conn {
	delay := minBackoff
	for {
		select {
		case <-time.After(delay):
		case <-c.ctx.Done():
			return nil
		}
		if c.ctx.Err() != nil {
			return nil
		}
		ws, welcome, err := handshake(c.ctx, c.socket, c.opts.Client)
		if err == nil {
			c.mu.Lock()
			if c.closed {
				c.mu.Unlock()
				ws.CloseNow()
				return nil
			}
			c.ws, c.welcome = ws, welcome
			close(c.up)
			c.mu.Unlock()
			c.logf("reconnected to %s", c.socket)
			c.restore(ws)
			return ws
		}
		c.logf("reconnect failed: %v", err)
		delay = min(delay*2, maxBackoff)
	}
}

// restore re-registers presence and resumes subscriptions on a fresh
// connection. It only writes; replies are handled by the reader, which has not
// started yet, so nothing is lost.
func (c *Client) restore(ws *websocket.Conn) {
	c.mu.Lock()
	presence := c.presence
	subs := make([]*Subscription, 0, len(c.subs))
	for _, s := range c.subs {
		subs = append(subs, s)
	}
	views := make([]*ViewSubscription, 0, len(c.views))
	for _, v := range c.views {
		views = append(views, v)
	}
	c.mu.Unlock()

	c.resend(ws, func(id string) protocol.EnvelopeVariant {
		return protocol.ConnectorsList{ID: id}
	}, func(err error) { c.logf("fetching connectors: %v", err) })
	if presence != nil {
		c.resend(ws, func(id string) protocol.EnvelopeVariant {
			return protocol.AppPresenceMessage{ID: &id, Payload: *presence}
		}, func(err error) { c.logf("restoring presence: %v", err) })
	}
	for _, s := range subs {
		ready, lastSeq := s.isReady()
		if !ready {
			continue // its Subscribe call failed with the old connection
		}
		from := lastSeq + 1
		c.resend(ws, func(id string) protocol.EnvelopeVariant {
			return protocol.SessionSubscribe{ID: id, Payload: protocol.SubscribeParams{Session: s.session, FromSeq: &from}}
		}, func(err error) {
			var rejected *Error
			if errors.As(err, &rejected) { // e.g. the session is gone
				c.mu.Lock()
				if c.subs[s.session] == s {
					delete(c.subs, s.session)
				}
				c.mu.Unlock()
				s.finish(err)
			}
		})
	}
	// A view has no seq to resume from: ask for a fresh snapshot, which the
	// subscription delivers as an update.
	for _, v := range views {
		if !v.isReady() {
			continue // its SubscribeView call failed with the old connection
		}
		c.resend(ws, func(id string) protocol.EnvelopeVariant {
			return protocol.ViewSubscribe{ID: id, Payload: protocol.SessionRef{Session: v.session}}
		}, func(err error) {
			var rejected *Error
			if errors.As(err, &rejected) { // e.g. the session is gone
				c.mu.Lock()
				if c.views[v.session] == v {
					delete(c.views, v.session)
				}
				c.mu.Unlock()
				v.finish(err)
			}
		})
	}
}

// resend sends a request nobody is waiting on and passes a failed reply to
// onErr from a goroutine.
func (c *Client) resend(ws *websocket.Conn, build func(id string) protocol.EnvelopeVariant, onErr func(error)) {
	c.mu.Lock()
	id, ch := c.registerLocked()
	c.mu.Unlock()
	if err := c.write(ws, build(id)); err != nil {
		c.forget(id) // the read loop will notice the dead connection
		return
	}
	go func() {
		if r := <-ch; r.err != nil {
			onErr(r.err)
		}
	}()
}

// shutdown marks the client finished and ends every subscription.
func (c *Client) shutdown(cause error) {
	c.mu.Lock()
	c.finished, c.err = true, cause
	err := cause
	subs, views := c.subs, c.views
	c.subs = map[string]*Subscription{}
	c.views = map[string]*ViewSubscription{}
	c.mu.Unlock()

	for _, s := range subs {
		s.finish(err)
	}
	for _, v := range views {
		v.finish(err)
	}
	close(c.done)
}
