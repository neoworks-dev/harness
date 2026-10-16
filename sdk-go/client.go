package sdk

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"sync"

	"github.com/coder/websocket"
	protocol "neoworks.dev/suite/protocol/go"
)

var (
	// ErrDaemonDown means nothing is listening on the socket.
	ErrDaemonDown = errors.New("daemon not running")
	// ErrConnectionLost means the connection ended while the client was open.
	ErrConnectionLost = errors.New("connection to daemon lost")
	// ErrClosed means Close was called on the client.
	ErrClosed = errors.New("client closed")
)

// Error is a bus.error reply: the daemon understood the request and refused it.
type Error struct{ protocol.ErrorInfo }

func (e *Error) Error() string { return e.Message }

// Options configure Dial. The zero value is usable.
type Options struct {
	// Socket is the daemon's Unix socket; empty means DefaultSocketPath().
	Socket string
	// Client names this client in bus.hello; empty means "go-sdk".
	Client string
	// Reconnect keeps the client alive across daemon restarts: it redials with
	// backoff, repeats the hello and the last SetPresence, and resumes each
	// subscription after the last event it saw. Requests in flight when the
	// connection drops fail with ErrConnectionLost and are not retried, since
	// the daemon may have processed them. Dial itself never retries.
	Reconnect bool
	// Logf receives reconnect diagnostics; nil discards them.
	Logf func(format string, args ...any)
	// OnFocus is called for every agent.focus the daemon sends: once after
	// each (re)connect and again on every change. Calls come in order from
	// one goroutine, so a slow callback delays later notifications (not the
	// connection) and may itself call the client.
	OnFocus func(protocol.AgentFocus)
	// OnConnectors is called with the full connector list whenever it changes,
	// and with the list fetched after each (re)connect. Same delivery as
	// OnFocus.
	OnConnectors func([]protocol.ConnectorInfo)
	// OnTheme is called with every theme the daemon sends: once after each
	// (re)connect and again when it changes. Same delivery as OnFocus.
	OnTheme func(protocol.Theme)
	// OnFileChanged is called for every file.changed the daemon passes on:
	// the agent's writes and other apps' saves (not this client's own).
	// Same delivery as OnFocus.
	OnFileChanged func(protocol.FileChanged)
}

// Client is a connection to the daemon.
type Client struct {
	opts   Options
	socket string
	ctx    context.Context // cancelled by Close; bounds reads, writes and redials
	cancel context.CancelFunc
	done   chan struct{} // closed when the client is finished

	mu       sync.Mutex
	ws       *websocket.Conn // nil while reconnecting
	up       chan struct{}   // closed while ws != nil
	welcome  protocol.Welcome
	nextID   int
	pending  map[string]chan reply
	subs     map[string]*Subscription // by session id
	views    map[string]*ViewSubscription
	presence *protocol.AppPresence    // re-sent after a reconnect
	focus    *protocol.AgentFocus     // latest agent.focus
	focused  chan struct{}            // closed by the first agent.focus
	conns    []protocol.ConnectorInfo // latest connector list
	theme    *protocol.Theme          // latest theme
	themed   chan struct{}            // closed by the first theme
	notes    *notifier
	closed   bool // Close was called
	finished bool // done is closed; err says why
	err      error
}

// reply is what a pending request receives: the reply frame, or the reason
// none will come.
type reply struct {
	v   protocol.EnvelopeVariant
	err error
}

// Dial connects to the daemon and completes the hello/welcome handshake.
// It returns ErrDaemonDown if nothing is listening.
func Dial(ctx context.Context, opts Options) (*Client, error) {
	socket := opts.Socket
	if socket == "" {
		socket = DefaultSocketPath()
	}
	if opts.Client == "" {
		opts.Client = "go-sdk"
	}
	ws, welcome, err := handshake(ctx, socket, opts.Client)
	if err != nil {
		return nil, err
	}
	c := &Client{
		opts:    opts,
		socket:  socket,
		done:    make(chan struct{}),
		ws:      ws,
		up:      make(chan struct{}),
		welcome: welcome,
		pending: map[string]chan reply{},
		subs:    map[string]*Subscription{},
		views:   map[string]*ViewSubscription{},
		focused: make(chan struct{}),
		themed:  make(chan struct{}),
		notes:   newNotifier(),
	}
	close(c.up)
	// Detached from ctx on purpose: ctx bounds the dial, Close ends the client.
	c.ctx, c.cancel = context.WithCancel(context.Background())
	go c.notes.run(c.ctx)
	go c.supervise(ws)
	go c.seedConnectors()
	return c, nil
}

// Focus returns the latest agent.focus: the daemon's current session and the
// app hosting the agent. The daemon sends one right after the handshake, so
// ok is false only for a moment after Dial; WaitFocus waits for it.
func (c *Client) Focus() (focus protocol.AgentFocus, ok bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.focus == nil {
		return protocol.AgentFocus{}, false
	}
	return *c.focus, true
}

// WaitFocus returns the current focus, waiting for the first agent.focus if
// it has not arrived yet.
func (c *Client) WaitFocus(ctx context.Context) (protocol.AgentFocus, error) {
	select {
	case <-c.focused:
	case <-c.done:
		return protocol.AgentFocus{}, c.Err()
	case <-ctx.Done():
		return protocol.AgentFocus{}, ctx.Err()
	}
	focus, _ := c.Focus()
	return focus, nil
}

// Connectors returns the latest connector list: fetched after each
// (re)connect and kept current from connectors.changed.
func (c *Client) Connectors() []protocol.ConnectorInfo {
	c.mu.Lock()
	defer c.mu.Unlock()
	return slices.Clone(c.conns)
}

// MoveAgent moves the agent to app (an open connector of kind app), or home
// when app is nil. A non-nil session makes that session current for every
// client. It returns once the daemon has broadcast the resulting agent.focus.
func (c *Client) MoveAgent(ctx context.Context, move protocol.AgentMove) error {
	_, err := request[protocol.BusAck](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.AgentMoveMessage{ID: id, Payload: move}
	})
	return err
}

// seedConnectors asks for the connector list; dispatch records the reply.
func (c *Client) seedConnectors() {
	ctx, cancel := context.WithTimeout(c.ctx, dialTimeout)
	defer cancel()
	c.ListConnectors(ctx) // a failure only means the list stays as it was
}

// Welcome is the daemon's handshake reply (from the latest connection).
func (c *Client) Welcome() protocol.Welcome {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.welcome
}

// Done is closed when the client can no longer be used.
func (c *Client) Done() <-chan struct{} { return c.done }

// Err says why Done closed: ErrClosed after Close, otherwise an error wrapping
// ErrConnectionLost. It is nil while the client is running.
func (c *Client) Err() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.err
}

// Close ends the connection. The daemon drops this client's presence with it.
func (c *Client) Close() error {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		<-c.done
		return nil
	}
	c.closed = true
	ws := c.ws
	c.mu.Unlock()

	// Close before cancelling: cancelling first would make the reader tear the
	// socket down without the closing handshake.
	if ws != nil {
		ws.Close(websocket.StatusNormalClosure, "")
	}
	c.cancel()
	<-c.done
	return nil
}

// ListSessions returns every session the daemon knows.
func (c *Client) ListSessions(ctx context.Context) ([]protocol.SessionInfo, error) {
	res, err := request[protocol.SessionListResult](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.SessionList{ID: id}
	})
	return res.Payload.Sessions, err
}

// CreateSession starts a new session.
func (c *Client) CreateSession(ctx context.Context, params protocol.SessionCreateParams) (protocol.SessionInfo, error) {
	res, err := request[protocol.SessionInfoMessage](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.SessionCreate{ID: id, Payload: params}
	})
	return res.Payload, err
}

// Subscribe replays the session's events from fromSeq (values below 1 mean
// from the start) and then follows it live. It returns once the replay has
// arrived: sub.Replay holds it and sub.Events() delivers what follows, so
// the boundary needs no in-band marker. Subscribing to a session twice on one
// client is an error.
func (c *Client) Subscribe(ctx context.Context, session string, fromSeq int64) (*Subscription, error) {
	return c.subscribe(ctx, session, fromSeq, false)
}

// SubscribeLive follows a session without replaying its history: sub.Replay
// is empty and sub.Info says where the log stands. For clients that only need
// to know what happens from now on (a session's running state, say).
func (c *Client) SubscribeLive(ctx context.Context, session string) (*Subscription, error) {
	return c.subscribe(ctx, session, 0, true)
}

// beyondLog is a from_seq past any real log: the daemon replays nothing.
const beyondLog = 1<<53 - 1

func (c *Client) subscribe(ctx context.Context, session string, fromSeq int64, live bool) (*Subscription, error) {
	c.mu.Lock()
	if c.subs[session] != nil {
		c.mu.Unlock()
		return nil, fmt.Errorf("already subscribed to session %s", session)
	}
	sub := newSubscription(c, session, fromSeq)
	// Registered before the request goes out: replayed events arrive first.
	c.subs[session] = sub
	c.mu.Unlock()

	params := protocol.SubscribeParams{Session: session}
	if live {
		from := int64(beyondLog)
		params.FromSeq = &from
	} else if fromSeq > 1 {
		params.FromSeq = &fromSeq
	}
	res, err := request[protocol.SessionSubscribed](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.SessionSubscribe{ID: id, Payload: params}
	})
	if err != nil {
		c.mu.Lock()
		if c.subs[session] == sub {
			delete(c.subs, session)
		}
		c.mu.Unlock()
		sub.stopped.Do(func() { close(sub.stop) })
		sub.finish(err)
		return nil, err
	}
	sub.Info = res.Payload.Session
	sub.Replay = sub.takeReplay()
	if live {
		// A resubscribe must resume after what the log held now, not replay it.
		sub.skipThrough(sub.Info.LastSeq)
	}
	return sub, nil
}

// Input submits text (and optionally images, as absolute paths) to a session.
// It returns once the daemon has queued it.
func (c *Client) Input(ctx context.Context, in protocol.SessionInput) error {
	_, err := request[protocol.BusAck](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.SessionInputMessage{ID: &id, Payload: in}
	})
	return err
}

// Cancel cancels the session's running turn. It is a no-op when idle.
func (c *Client) Cancel(ctx context.Context, session string) error {
	_, err := request[protocol.BusAck](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.SessionCancel{ID: id, Payload: protocol.SessionRef{Session: session}}
	})
	return err
}

// ListConnectors returns every connector the daemon knows, open or not.
func (c *Client) ListConnectors(ctx context.Context) ([]protocol.ConnectorInfo, error) {
	res, err := request[protocol.ConnectorsListResult](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.ConnectorsList{ID: id}
	})
	return res.Payload.Connectors, err
}

// SetPresence registers (p.Open) or unregisters this client as a connector,
// or updates its registration. A registration lasts as long as the
// connection; with Options.Reconnect the latest one is re-sent after each
// reconnect.
func (c *Client) SetPresence(ctx context.Context, p protocol.AppPresence) error {
	c.mu.Lock()
	prev := c.presence
	c.presence = nil
	if p.Open {
		c.presence = &p
	}
	c.mu.Unlock()

	_, err := request[protocol.BusAck](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.AppPresenceMessage{ID: &id, Payload: p}
	})
	var rejected *Error
	if errors.As(err, &rejected) {
		// Don't re-send a registration the daemon refuses on every reconnect.
		c.mu.Lock()
		c.presence = prev
		c.mu.Unlock()
	}
	return err
}

// request sends the message built for a fresh id and returns the reply as R.
func request[R protocol.EnvelopeVariant](ctx context.Context, c *Client, build func(id string) protocol.EnvelopeVariant) (R, error) {
	var zero R
	v, err := c.call(ctx, build)
	if err != nil {
		return zero, err
	}
	r, ok := v.(R)
	if !ok {
		return zero, fmt.Errorf("unexpected reply %T", v)
	}
	return r, nil
}

// call sends one request and waits for the reply with the same id. While the
// client is reconnecting it waits for the connection (until ctx ends).
func (c *Client) call(ctx context.Context, build func(id string) protocol.EnvelopeVariant) (protocol.EnvelopeVariant, error) {
	id, ws, ch, err := c.begin(ctx)
	if err != nil {
		return nil, err
	}
	if err := c.write(ws, build(id)); err != nil {
		c.forget(id)
		return nil, c.lostError(err)
	}
	select {
	case r := <-ch:
		return r.v, r.err
	case <-ctx.Done():
		c.forget(id)
		return nil, ctx.Err()
	}
}

// begin waits for a live connection and registers a pending request on it.
func (c *Client) begin(ctx context.Context) (string, *websocket.Conn, chan reply, error) {
	for {
		c.mu.Lock()
		if c.finished {
			err := c.err
			c.mu.Unlock()
			return "", nil, nil, err
		}
		if ws := c.ws; ws != nil {
			id, ch := c.registerLocked()
			c.mu.Unlock()
			return id, ws, ch, nil
		}
		up := c.up
		c.mu.Unlock()
		select {
		case <-up:
		case <-c.done:
		case <-ctx.Done():
			return "", nil, nil, ctx.Err()
		}
	}
}

// registerLocked allocates a request id and a channel for its reply.
func (c *Client) registerLocked() (string, chan reply) {
	c.nextID++
	id := strconv.Itoa(c.nextID)
	ch := make(chan reply, 1)
	c.pending[id] = ch
	return id, ch
}

func (c *Client) forget(id string) {
	c.mu.Lock()
	delete(c.pending, id)
	c.mu.Unlock()
}

// lostError describes why a request or the connection failed.
func (c *Client) lostError(cause error) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return ErrClosed
	}
	return fmt.Errorf("%w: %v", ErrConnectionLost, cause)
}

func (c *Client) logf(format string, args ...any) {
	if c.opts.Logf != nil {
		c.opts.Logf(format, args...)
	}
}
