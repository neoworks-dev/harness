package sdktest

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	protocol "neoworks.dev/suite/protocol/go"
)

// Daemon is a fake daemon listening on a Unix socket.
type Daemon struct {
	// Socket is the path clients dial.
	Socket string
	// StartedAt is reported in bus.welcome; it defaults to 90 minutes ago.
	StartedAt time.Time
	// StepDelay separates the events of a scripted agent turn.
	StepDelay time.Duration
	// AgentReply scripts the agent's events for an input.
	AgentReply func(text string) []protocol.SessionEventVariant

	t   testing.TB
	srv *http.Server

	mu         sync.Mutex
	sessions   map[string]*session
	nextID     int
	hellos     []protocol.Hello
	inputs     []protocol.SessionInput
	cancels    []string
	subscribes []protocol.SubscribeParams
	conns      map[*websocket.Conn]*connState
	connectors map[string]*protocol.ConnectorInfo
	presences  map[string]protocol.AppPresence // open connectors only
	moves      []protocol.AgentMove
	current    string // the current session; empty until first needed
	focusApp   string // app hosting the agent; empty when none
}

type session struct {
	info    protocol.SessionInfo
	log     []protocol.SessionEventRecord
	subs    []*websocket.Conn
	queue   []protocol.SessionInput
	working bool
}

// connState is what the daemon remembers about one client connection.
type connState struct {
	hello bool
	app   string // connector registered by this connection, if any
}

// New starts a fake daemon that is stopped when the test ends.
func New(t testing.TB) *Daemon {
	t.Helper()
	dir, err := os.MkdirTemp("", "suite")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	d := &Daemon{
		Socket:    filepath.Join(dir, "bus.sock"),
		StartedAt: time.Now().Add(-90 * time.Minute),
		StepDelay: 30 * time.Millisecond,
		AgentReply: func(text string) []protocol.SessionEventVariant {
			return []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m1", Text: "you said: " + text},
				protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
			}
		},
		t:          t,
		sessions:   map[string]*session{},
		conns:      map[*websocket.Conn]*connState{},
		connectors: map[string]*protocol.ConnectorInfo{},
		presences:  map[string]protocol.AppPresence{},
	}
	d.Start()
	t.Cleanup(d.Stop)
	return d
}

// Start listens on the socket again after Stop. State (sessions, connectors)
// survives, as it would across a daemon restart with its store on disk;
// connector presence does not, because it lives and dies with a connection.
func (d *Daemon) Start() {
	d.t.Helper()
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.srv != nil {
		d.t.Fatal("sdktest: already started")
	}
	l, err := net.Listen("unix", d.Socket)
	if err != nil {
		d.t.Fatal(err)
	}
	d.srv = &http.Server{Handler: http.HandlerFunc(d.serve)}
	go d.srv.Serve(l)
}

// Stop closes the listener and every connection.
func (d *Daemon) Stop() {
	d.mu.Lock()
	srv := d.srv
	d.srv = nil
	d.mu.Unlock()
	if srv != nil {
		srv.Close()
	}
	d.DropConnections()
}

// DropConnections closes every client connection, as a crashing daemon would,
// but keeps listening.
func (d *Daemon) DropConnections() {
	d.mu.Lock()
	defer d.mu.Unlock()
	for c := range d.conns {
		c.CloseNow()
	}
}

// AddSession registers a session whose log already holds events.
func (d *Daemon) AddSession(id, title string, lastActivity time.Time, events ...protocol.SessionEventVariant) {
	d.mu.Lock()
	defer d.mu.Unlock()
	s := &session{info: protocol.SessionInfo{ID: id, Cwd: "/work/" + id, State: protocol.SessionStateIdle}}
	if title != "" {
		s.info.Title = &title
	}
	for _, ev := range events {
		d.appendLocked(s, ev)
	}
	s.info.LastActivity = lastActivity.UnixMilli()
	d.sessions[id] = s
}

// Forget deletes a session, as if it were removed while the daemon was down.
func (d *Daemon) Forget(id string) {
	d.mu.Lock()
	defer d.mu.Unlock()
	delete(d.sessions, id)
}

// SetSessionState overrides what session.list reports for a session.
func (d *Daemon) SetSessionState(id string, state protocol.SessionState, queued int64) {
	d.mu.Lock()
	defer d.mu.Unlock()
	s := d.sessions[id]
	s.info.State, s.info.Queued = state, queued
}

// Append adds an event to a session's log and streams it to subscribers, as
// if another client had caused it.
func (d *Daemon) Append(id string, ev protocol.SessionEventVariant) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.appendLocked(d.sessions[id], ev)
}

// Session returns a session's current info.
func (d *Daemon) Session(id string) (protocol.SessionInfo, bool) {
	d.mu.Lock()
	defer d.mu.Unlock()
	s := d.sessions[id]
	if s == nil {
		return protocol.SessionInfo{}, false
	}
	return s.info, true
}

// SetConnector makes connectors.list report c (for tests that need tools
// without running an app). A later presence for the same app keeps its tools.
func (d *Daemon) SetConnector(c protocol.ConnectorInfo) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.connectors[c.App] = &c
	d.broadcastConnectorsLocked()
}

// Focus returns where the agent is: the current session (created on demand,
// like the real daemon's) and the hosting app, if any.
func (d *Daemon) Focus() (session string, app *string) {
	d.mu.Lock()
	defer d.mu.Unlock()
	session = d.ensureCurrentLocked()
	if d.focusApp != "" {
		app = &d.focusApp
	}
	return session, app
}

// MoveAgentTo moves the agent as if it had called apps_focus (reason "agent")
// or otherwise changed on its own, and broadcasts agent.focus. An empty app
// means no app.
func (d *Daemon) MoveAgentTo(app string, reason protocol.FocusReason) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.setFocusLocked(app, reason)
}

// Moves returns the payload of every agent.move received.
func (d *Daemon) Moves() []protocol.AgentMove {
	d.mu.Lock()
	defer d.mu.Unlock()
	return slices.Clone(d.moves)
}

// Presence returns the registration of an open connector.
func (d *Daemon) Presence(app string) (protocol.AppPresence, bool) {
	d.mu.Lock()
	defer d.mu.Unlock()
	p, ok := d.presences[app]
	return p, ok
}

// Inputs returns every session.input received, in order.
func (d *Daemon) Inputs() []protocol.SessionInput {
	d.mu.Lock()
	defer d.mu.Unlock()
	return slices.Clone(d.inputs)
}

// Cancels returns the session ids session.cancel was called for.
func (d *Daemon) Cancels() []string {
	d.mu.Lock()
	defer d.mu.Unlock()
	return slices.Clone(d.cancels)
}

// Hellos returns the payload of every bus.hello received.
func (d *Daemon) Hellos() []protocol.Hello {
	d.mu.Lock()
	defer d.mu.Unlock()
	return slices.Clone(d.hellos)
}

// Subscribes returns the params of every session.subscribe received.
func (d *Daemon) Subscribes() []protocol.SubscribeParams {
	d.mu.Lock()
	defer d.mu.Unlock()
	return slices.Clone(d.subscribes)
}

// Eventually polls cond until it holds, failing the test after five seconds.
func Eventually(t testing.TB, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !cond() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func (d *Daemon) appendLocked(s *session, ev protocol.SessionEventVariant) {
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

func (d *Daemon) write(c *websocket.Conn, v protocol.EnvelopeVariant) {
	data, err := json.Marshal(protocol.Envelope{Value: v})
	if err != nil {
		d.t.Error(err)
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c.Write(ctx, websocket.MessageText, data) // a closed client is fine
}

func (d *Daemon) fail(c *websocket.Conn, id *string, code protocol.ErrorCode, message string) {
	d.write(c, protocol.BusError{ID: id, Payload: protocol.ErrorInfo{Code: code, Message: message}})
}

func (d *Daemon) serve(w http.ResponseWriter, r *http.Request) {
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
	d.conns[c] = &connState{}
	d.mu.Unlock()
	defer d.disconnect(c)

	for {
		_, data, err := c.Read(r.Context())
		if err != nil {
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

// disconnect forgets a connection: its subscriptions and its connector.
func (d *Daemon) disconnect(c *websocket.Conn) {
	d.mu.Lock()
	defer d.mu.Unlock()
	for _, s := range d.sessions {
		s.subs = slices.DeleteFunc(s.subs, func(sc *websocket.Conn) bool { return sc == c })
	}
	if st := d.conns[c]; st != nil && st.app != "" {
		d.unregisterLocked(st.app)
	}
	delete(d.conns, c)
}

func (d *Daemon) unregisterLocked(app string) {
	delete(d.presences, app)
	if info := d.connectors[app]; info != nil {
		info.Connected = false
	}
	if d.focusApp == app {
		d.setFocusLocked(d.homeLocked(), protocol.FocusReasonClosed)
	}
	d.broadcastConnectorsLocked()
}

// homeLocked returns the open connector registered with home, or "".
func (d *Daemon) homeLocked() string {
	for app, p := range d.presences {
		if p.Home != nil && *p.Home {
			return app
		}
	}
	return ""
}

// ensureCurrentLocked returns the current session, picking the most recently
// active one or creating one, as the real daemon always has one.
func (d *Daemon) ensureCurrentLocked() string {
	if _, ok := d.sessions[d.current]; ok {
		return d.current
	}
	var best *session
	for _, s := range d.sessions {
		// Ties go to the greater id, so the choice is stable.
		if best == nil || s.info.LastActivity > best.info.LastActivity ||
			s.info.LastActivity == best.info.LastActivity && s.info.ID > best.info.ID {
			best = s
		}
	}
	if best == nil {
		id := "main"
		now := time.Now().UnixMilli()
		best = &session{info: protocol.SessionInfo{ID: id, CreatedAt: now, LastActivity: now, State: protocol.SessionStateIdle}}
		d.sessions[id] = best
	}
	d.current = best.info.ID
	return d.current
}

func (d *Daemon) focusLocked(reason protocol.FocusReason, from string) protocol.AgentFocus {
	f := protocol.AgentFocus{Session: d.ensureCurrentLocked(), Reason: reason}
	if d.focusApp != "" {
		f.App = &d.focusApp
	}
	if from != "" && from != d.focusApp {
		f.From = &from
	}
	return f
}

// setFocusLocked moves the agent to app ("" for none) and broadcasts it. A
// move to where it already is sends nothing.
func (d *Daemon) setFocusLocked(app string, reason protocol.FocusReason) {
	if app == d.focusApp {
		return
	}
	from := d.focusApp
	d.focusApp = app
	d.broadcastLocked(protocol.AgentFocusMessage{Payload: d.focusLocked(reason, from)})
}

func (d *Daemon) broadcastLocked(v protocol.EnvelopeVariant) {
	for c, st := range d.conns {
		if st.hello {
			d.write(c, v)
		}
	}
}

func (d *Daemon) broadcastConnectorsLocked() {
	d.broadcastLocked(protocol.ConnectorsChanged{Payload: protocol.ConnectorsListPayload{Connectors: d.connectorListLocked()}})
}

func (d *Daemon) connectorListLocked() []protocol.ConnectorInfo {
	list := []protocol.ConnectorInfo{}
	for _, info := range d.connectors {
		list = append(list, *info)
	}
	slices.SortFunc(list, func(a, b protocol.ConnectorInfo) int { return strings.Compare(a.App, b.App) })
	return list
}

func (d *Daemon) handle(c *websocket.Conn, v protocol.EnvelopeVariant) {
	d.mu.Lock()
	defer d.mu.Unlock()
	st := d.conns[c]
	if st == nil {
		return // dropped meanwhile
	}
	if _, isHello := v.(protocol.BusHello); !isHello && !st.hello {
		d.fail(c, nil, protocol.ErrorCodeBadRequest, "send bus.hello first")
		return
	}
	switch m := v.(type) {
	case protocol.BusHello:
		d.hellos = append(d.hellos, m.Payload)
		if m.Payload.Protocol != 1 {
			d.fail(c, nil, protocol.ErrorCodeUnsupportedProtocol, fmt.Sprintf("daemon speaks protocol 1, client sent %d", m.Payload.Protocol))
			return
		}
		st.hello = true
		d.write(c, protocol.BusWelcome{Payload: protocol.Welcome{
			Protocol: 1, DaemonVersion: "test", PID: 4242, StartedAt: d.StartedAt.UnixMilli(),
		}})
		d.write(c, protocol.AgentFocusMessage{Payload: d.focusLocked(protocol.FocusReasonInitial, "")})
	case protocol.SessionCreate:
		d.nextID++
		id := fmt.Sprintf("new%d", d.nextID)
		s := &session{info: protocol.SessionInfo{
			ID: id, Title: m.Payload.Title, CreatedAt: time.Now().UnixMilli(),
			LastActivity: time.Now().UnixMilli(), State: protocol.SessionStateIdle,
		}}
		if m.Payload.Cwd != nil {
			s.info.Cwd = *m.Payload.Cwd
		}
		d.sessions[id] = s
		d.write(c, protocol.SessionInfoMessage{ID: m.ID, Payload: s.info})
	case protocol.SessionList:
		list := []protocol.SessionInfo{}
		for _, s := range d.sessions {
			list = append(list, s.info)
		}
		d.write(c, protocol.SessionListResult{ID: m.ID, Payload: protocol.SessionListPayload{Sessions: list}})
	case protocol.SessionSubscribe:
		d.subscribes = append(d.subscribes, m.Payload)
		s := d.sessions[m.Payload.Session]
		if s == nil {
			d.fail(c, &m.ID, protocol.ErrorCodeNotFound, "no session "+m.Payload.Session)
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
		if !slices.Contains(s.subs, c) {
			s.subs = append(s.subs, c)
		}
	case protocol.SessionUnsubscribe:
		if s := d.sessions[m.Payload.Session]; s != nil {
			s.subs = slices.DeleteFunc(s.subs, func(sc *websocket.Conn) bool { return sc == c })
		}
		d.write(c, protocol.BusAck{ID: m.ID})
	case protocol.SessionInputMessage:
		s := d.sessions[m.Payload.Session]
		if s == nil {
			d.fail(c, m.ID, protocol.ErrorCodeNotFound, "no session "+m.Payload.Session)
			return
		}
		d.inputs = append(d.inputs, m.Payload)
		// The session is running or queued before the ack goes out.
		if s.working {
			s.queue = append(s.queue, m.Payload)
			s.info.Queued = int64(len(s.queue))
		} else {
			s.working = true
			s.info.State = protocol.SessionStateRunning
			go d.runTurns(s, m.Payload)
		}
		if m.ID != nil {
			d.write(c, protocol.BusAck{ID: *m.ID})
		}
	case protocol.SessionCancel:
		d.cancels = append(d.cancels, m.Payload.Session)
		d.write(c, protocol.BusAck{ID: m.ID})
	case protocol.AppPresenceMessage:
		d.presence(c, st, m)
	case protocol.ConnectorsList:
		d.write(c, protocol.ConnectorsListResult{ID: m.ID, Payload: protocol.ConnectorsListPayload{Connectors: d.connectorListLocked()}})
	case protocol.AgentMoveMessage:
		d.move(c, m)
	}
}

// move handles agent.move: the focus broadcast goes out before the ack.
func (d *Daemon) move(c *websocket.Conn, m protocol.AgentMoveMessage) {
	d.moves = append(d.moves, m.Payload)
	app := d.homeLocked()
	if m.Payload.App != nil {
		app = *m.Payload.App
		p, open := d.presences[app]
		switch {
		case !open:
			d.fail(c, &m.ID, protocol.ErrorCodeNotFound, app+" is not open")
			return
		case p.Kind != protocol.ConnectorKindApp:
			d.fail(c, &m.ID, protocol.ErrorCodeBadRequest, app+" is not an app")
			return
		}
	}
	if sid := m.Payload.Session; sid != nil {
		if d.sessions[*sid] == nil {
			d.fail(c, &m.ID, protocol.ErrorCodeNotFound, "no session "+*sid)
			return
		}
		if *sid != d.ensureCurrentLocked() {
			from := d.focusApp
			d.current = *sid
			d.focusApp = app
			d.broadcastLocked(protocol.AgentFocusMessage{Payload: d.focusLocked(protocol.FocusReasonSession, from)})
			d.write(c, protocol.BusAck{ID: m.ID})
			return
		}
	}
	d.setFocusLocked(app, protocol.FocusReasonUser)
	d.write(c, protocol.BusAck{ID: m.ID})
}

// presence registers or unregisters a connector. A connection holds at most
// one, like the real daemon's one-app-per-client model.
func (d *Daemon) presence(c *websocket.Conn, st *connState, m protocol.AppPresenceMessage) {
	p := m.Payload
	ack := func() {
		if m.ID != nil {
			d.write(c, protocol.BusAck{ID: *m.ID})
		}
	}
	if p.App == "" {
		d.fail(c, m.ID, protocol.ErrorCodeBadRequest, "app is required")
		return
	}
	if st.app != "" && st.app != p.App {
		d.fail(c, m.ID, protocol.ErrorCodeBadRequest, "connection already registered as "+st.app)
		return
	}
	if !p.Open {
		d.unregisterLocked(p.App)
		st.app = ""
		ack()
		return
	}
	info := d.connectors[p.App]
	if info == nil {
		info = &protocol.ConnectorInfo{App: p.App, Tools: []protocol.ConnectorTool{}}
		d.connectors[p.App] = info
	}
	info.Kind, info.Title, info.Connected = p.Kind, p.Title, true
	info.Home, info.PID, info.Context = p.Home, p.PID, p.Context
	info.V++
	d.presences[p.App] = p
	st.app = p.App
	// The home app takes an agent that has nowhere to be.
	if d.focusApp == "" && p.Home != nil && *p.Home {
		d.setFocusLocked(p.App, protocol.FocusReasonOpened)
	}
	d.broadcastConnectorsLocked()
	ack()
}

// runTurns plays the scripted agent for in and then for anything queued.
func (d *Daemon) runTurns(s *session, in protocol.SessionInput) {
	for {
		d.mu.Lock()
		d.appendLocked(s, protocol.UserMessage{App: in.App, Text: in.Text, Images: in.Images})
		reply := d.AgentReply(in.Text)
		d.mu.Unlock()
		for _, ev := range reply {
			time.Sleep(d.StepDelay)
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
		in, s.queue = s.queue[0], s.queue[1:]
		s.info.Queued = int64(len(s.queue))
		d.mu.Unlock()
	}
}
