package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	protocol "neoworks.dev/suite/protocol/go"
)

// fakeDaemon speaks enough of the bus protocol to exercise the CLI: sessions
// with an event log, replay-then-live subscriptions, and a scripted agent that
// answers each input after a short delay.
type fakeDaemon struct {
	t          *testing.T
	socket     string
	startedAt  time.Time
	stepDelay  time.Duration
	mu         sync.Mutex
	sessions   map[string]*fakeSession
	nextID     int
	inputs     []protocol.SessionInput // every input received, in order
	cancels    []string                // session ids cancel was called for
	conns      []*websocket.Conn
	agentReply func(text string) []protocol.SessionEventVariant
}

type fakeSession struct {
	info    protocol.SessionInfo
	log     []protocol.SessionEventRecord
	subs    []*websocket.Conn
	queue   []string
	working bool
}

func newFakeDaemon(t *testing.T) *fakeDaemon {
	t.Helper()
	dir, err := os.MkdirTemp("", "suite")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	d := &fakeDaemon{
		t:         t,
		socket:    filepath.Join(dir, "bus.sock"),
		startedAt: time.Now().Add(-90 * time.Minute),
		stepDelay: 30 * time.Millisecond,
		sessions:  map[string]*fakeSession{},
		agentReply: func(text string) []protocol.SessionEventVariant {
			return []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m1", Text: "you said: " + text},
				protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
			}
		},
	}
	l, err := net.Listen("unix", d.socket)
	if err != nil {
		t.Fatal(err)
	}
	srv := &http.Server{Handler: http.HandlerFunc(d.serve)}
	go srv.Serve(l)
	t.Cleanup(func() { srv.Close() })
	return d
}

// addSession registers a session whose log already holds events.
func (d *fakeDaemon) addSession(id string, title string, lastActivity time.Time, events ...protocol.SessionEventVariant) *fakeSession {
	d.mu.Lock()
	defer d.mu.Unlock()
	s := &fakeSession{info: protocol.SessionInfo{ID: id, Cwd: "/work/" + id, State: protocol.SessionStateIdle}}
	if title != "" {
		s.info.Title = &title
	}
	for _, ev := range events {
		d.appendLocked(s, ev)
	}
	s.info.LastActivity = lastActivity.UnixMilli()
	d.sessions[id] = s
	return s
}

func (d *fakeDaemon) appendLocked(s *fakeSession, ev protocol.SessionEventVariant) {
	rec := protocol.SessionEventRecord{
		Session: s.info.ID, Seq: int64(len(s.log) + 1), Ts: time.Now().UnixMilli(),
		Event: protocol.SessionEvent{Value: ev},
	}
	s.log = append(s.log, rec)
	s.info.LastSeq = rec.Seq
	s.info.LastActivity = rec.Ts
	for _, c := range s.subs {
		d.write(c, protocol.SessionEvents{Payload: rec})
	}
}

func (d *fakeDaemon) write(c *websocket.Conn, v protocol.EnvelopeVariant) {
	data, err := json.Marshal(protocol.Envelope{Value: v})
	if err != nil {
		d.t.Error(err)
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c.Write(ctx, websocket.MessageText, data) // a closed client is fine
}

func (d *fakeDaemon) serve(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/bus" {
		http.NotFound(w, r)
		return
	}
	c, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	defer c.CloseNow()
	d.mu.Lock()
	d.conns = append(d.conns, c)
	d.mu.Unlock()
	ctx := r.Context()
	for {
		_, data, err := c.Read(ctx)
		if err != nil {
			d.unsubscribe(c)
			return
		}
		var env protocol.Envelope
		if err := json.Unmarshal(data, &env); err != nil {
			d.t.Errorf("fake daemon: bad frame %s: %v", data, err)
			return
		}
		d.handle(c, env.Value)
	}
}

// dropConnections closes every client connection, as a crashing daemon would.
func (d *fakeDaemon) dropConnections() {
	d.mu.Lock()
	defer d.mu.Unlock()
	for _, c := range d.conns {
		c.CloseNow()
	}
}

func (d *fakeDaemon) unsubscribe(c *websocket.Conn) {
	d.mu.Lock()
	defer d.mu.Unlock()
	for _, s := range d.sessions {
		for i, sc := range s.subs {
			if sc == c {
				s.subs = append(s.subs[:i], s.subs[i+1:]...)
				break
			}
		}
	}
}

func (d *fakeDaemon) notFound(c *websocket.Conn, id string, session string) {
	d.write(c, protocol.BusError{ID: &id, Payload: protocol.ErrorInfo{Code: protocol.ErrorCodeNotFound, Message: "no such session: " + session}})
}

func (d *fakeDaemon) handle(c *websocket.Conn, v protocol.EnvelopeVariant) {
	d.mu.Lock()
	defer d.mu.Unlock()
	switch m := v.(type) {
	case protocol.BusHello:
		d.write(c, protocol.BusWelcome{Payload: protocol.Welcome{
			Protocol: 1, DaemonVersion: "test", PID: 4242, StartedAt: d.startedAt.UnixMilli(),
		}})
	case protocol.SessionCreate:
		d.nextID++
		id := fmt.Sprintf("new%d", d.nextID)
		s := &fakeSession{info: protocol.SessionInfo{
			ID: id, Title: m.Payload.Title, CreatedAt: time.Now().UnixMilli(),
			LastActivity: time.Now().UnixMilli(), State: protocol.SessionStateIdle,
		}}
		if m.Payload.Cwd != nil {
			s.info.Cwd = *m.Payload.Cwd
		}
		d.sessions[id] = s
		d.write(c, protocol.SessionInfoMessage{ID: m.ID, Payload: s.info})
	case protocol.SessionList:
		var list []protocol.SessionInfo
		for _, s := range d.sessions {
			list = append(list, s.info)
		}
		d.write(c, protocol.SessionListResult{ID: m.ID, Payload: protocol.SessionListPayload{Sessions: list}})
	case protocol.SessionSubscribe:
		s := d.sessions[m.Payload.Session]
		if s == nil {
			d.notFound(c, m.ID, m.Payload.Session)
			return
		}
		from := int64(1)
		if m.Payload.FromSeq != nil {
			from = *m.Payload.FromSeq
		}
		// Replay, the marker and registering for live events happen under one
		// lock, so nothing is missed or sent twice.
		for _, rec := range s.log {
			if rec.Seq >= from {
				d.write(c, protocol.SessionEvents{Payload: rec})
			}
		}
		d.write(c, protocol.SessionSubscribed{ID: m.ID, Payload: protocol.SubscribedPayload{Session: s.info, ReplayedThrough: s.info.LastSeq}})
		s.subs = append(s.subs, c)
	case protocol.SessionInputMessage:
		s := d.sessions[m.Payload.Session]
		if s == nil {
			if m.ID != nil {
				d.notFound(c, *m.ID, m.Payload.Session)
			}
			return
		}
		d.inputs = append(d.inputs, m.Payload)
		// The session is running or queued before the ack goes out.
		if s.working {
			s.queue = append(s.queue, m.Payload.Text)
			s.info.Queued = int64(len(s.queue))
		} else {
			s.working = true
			s.info.State = protocol.SessionStateRunning
			go d.runTurns(s, m.Payload.App, m.Payload.Text)
		}
		if m.ID != nil {
			d.write(c, protocol.BusAck{ID: *m.ID})
		}
	case protocol.SessionCancel:
		d.cancels = append(d.cancels, m.Payload.Session)
		d.write(c, protocol.BusAck{ID: m.ID})
	}
}

// runTurns plays the scripted agent for text and then for anything queued.
func (d *fakeDaemon) runTurns(s *fakeSession, app, text string) {
	for {
		d.mu.Lock()
		d.appendLocked(s, protocol.UserMessage{App: app, Text: text})
		reply := d.agentReply(text)
		d.mu.Unlock()
		for _, ev := range reply {
			time.Sleep(d.stepDelay)
			d.mu.Lock()
			d.appendLocked(s, ev)
			d.mu.Unlock()
		}
		d.mu.Lock()
		if len(s.queue) == 0 {
			s.working = false
			s.info.State = protocol.SessionStateIdle
			d.mu.Unlock()
			return
		}
		text, s.queue = s.queue[0], s.queue[1:]
		s.info.Queued = int64(len(s.queue))
		d.mu.Unlock()
	}
}
