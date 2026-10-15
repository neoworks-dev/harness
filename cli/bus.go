package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"net"
	"net/http"
	"strconv"
	"syscall"
	"time"

	"github.com/coder/websocket"
	protocol "neoworks.dev/suite/protocol/go"
)

const (
	protocolVersion = 1
	// Events carry tool output and can be large.
	readLimit   = 16 << 20
	dialTimeout = 5 * time.Second
)

// errDaemonDown means nothing is listening on the socket.
var errDaemonDown = errors.New("daemon not running")

// frame is one decoded envelope from the daemon, or the error that ended the
// connection (always the last frame).
type frame struct {
	value protocol.EnvelopeVariant
	err   error
}

// bus is a connection to the daemon. A reader goroutine decodes frames onto
// in, in wire order, so callers can drive everything from one select loop and
// need no locking. Writes go through send.
type bus struct {
	ws      *websocket.Conn
	in      chan frame
	cancel  context.CancelFunc
	nextID  int
	welcome protocol.Welcome
}

// dialBus connects to the unix socket, upgrades to WebSocket at /bus and
// completes the hello/welcome handshake.
func dialBus(ctx context.Context, socket string) (*bus, error) {
	dialCtx, cancelDial := context.WithTimeout(ctx, dialTimeout)
	defer cancelDial()

	client := &http.Client{Transport: &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "unix", socket)
		},
	}}
	// The host is ignored by the dialer above; it only has to be well-formed.
	ws, _, err := websocket.Dial(dialCtx, "ws://suite/bus", &websocket.DialOptions{HTTPClient: client})
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) || errors.Is(err, syscall.ECONNREFUSED) {
			return nil, errDaemonDown
		}
		return nil, err
	}
	ws.SetReadLimit(readLimit)

	readCtx, cancel := context.WithCancel(ctx)
	b := &bus{ws: ws, in: make(chan frame), cancel: cancel}
	go b.readLoop(readCtx)

	if err := b.send(dialCtx, protocol.BusHello{Payload: protocol.Hello{Protocol: protocolVersion, Client: "cli"}}); err != nil {
		b.close()
		return nil, err
	}
	for {
		select {
		case f := <-b.in:
			if f.err != nil {
				b.close()
				return nil, f.err
			}
			switch v := f.value.(type) {
			case protocol.BusWelcome:
				b.welcome = v.Payload
				return b, nil
			case protocol.BusError:
				b.close()
				return nil, errors.New(v.Payload.Message)
			}
		case <-dialCtx.Done():
			b.close()
			return nil, fmt.Errorf("no welcome from daemon: %w", dialCtx.Err())
		}
	}
}

func (b *bus) readLoop(ctx context.Context) {
	defer close(b.in)
	for {
		_, data, err := b.ws.Read(ctx)
		if err != nil {
			select {
			case b.in <- frame{err: err}:
			case <-ctx.Done():
			}
			return
		}
		var env protocol.Envelope
		if json.Unmarshal(data, &env) != nil || env.Value == nil {
			continue // not a frame we can use
		}
		select {
		case b.in <- frame{value: env.Value}:
		case <-ctx.Done():
			return
		}
	}
}

func (b *bus) send(ctx context.Context, v protocol.EnvelopeVariant) error {
	data, err := json.Marshal(protocol.Envelope{Value: v})
	if err != nil {
		return err
	}
	return b.ws.Write(ctx, websocket.MessageText, data)
}

func (b *bus) newID() string {
	b.nextID++
	return strconv.Itoa(b.nextID)
}

func (b *bus) close() {
	b.cancel()
	b.ws.Close(websocket.StatusNormalClosure, "")
}

// busError is a bus.error reply.
type busError struct{ info protocol.ErrorInfo }

func (e *busError) Error() string { return e.info.Message }

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
	case protocol.SessionSubscribed:
		return m.ID, true
	}
	return "", false
}

// call sends the request built for a fresh id and waits for its reply,
// dropping unrelated frames. Only for use before subscribing, when nothing
// else we care about is in flight.
func (b *bus) call(ctx context.Context, build func(id string) protocol.EnvelopeVariant) (protocol.EnvelopeVariant, error) {
	id := b.newID()
	if err := b.send(ctx, build(id)); err != nil {
		return nil, err
	}
	for {
		select {
		case f, ok := <-b.in:
			if !ok || f.err != nil {
				return nil, errors.New("connection to daemon lost")
			}
			if rid, ok := replyID(f.value); ok && rid == id {
				if e, isErr := f.value.(protocol.BusError); isErr {
					return nil, &busError{e.Payload}
				}
				return f.value, nil
			}
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
}

func (b *bus) listSessions(ctx context.Context) ([]protocol.SessionInfo, error) {
	reply, err := b.call(ctx, func(id string) protocol.EnvelopeVariant { return protocol.SessionList{ID: id} })
	if err != nil {
		return nil, err
	}
	res, ok := reply.(protocol.SessionListResult)
	if !ok {
		return nil, fmt.Errorf("unexpected reply %T to session.list", reply)
	}
	return res.Payload.Sessions, nil
}
