package sdk

import (
	"context"
	"sync"
	"time"

	protocol "neoworks.dev/neo/protocol/go"
)

// Subscription follows one session's event log: the replay requested at
// Subscribe time, then live events.
type Subscription struct {
	// Info is the session as it was when the subscription started.
	Info protocol.SessionInfo
	// Replay holds the events from the requested seq through Info.LastSeq, in
	// order. Everything on Events is newer.
	Replay []protocol.SessionEventRecord

	c       *Client
	session string
	out     chan protocol.SessionEventRecord
	wake    chan struct{} // cap 1: the queue has something new
	stop    chan struct{} // closed by Close: abandon undelivered events
	stopped sync.Once

	mu      sync.Mutex
	ready   bool // the daemon finished the replay; later events are live
	lastSeq int64
	replay  []protocol.SessionEventRecord // replay being collected
	queue   []protocol.SessionEventRecord // live events not yet delivered
	ended   bool
	err     error
}

func newSubscription(c *Client, session string, fromSeq int64) *Subscription {
	s := &Subscription{
		c:       c,
		session: session,
		out:     make(chan protocol.SessionEventRecord),
		wake:    make(chan struct{}, 1),
		stop:    make(chan struct{}),
		lastSeq: max(fromSeq, 1) - 1,
	}
	go s.pump()
	return s
}

// Events delivers live events in seq order, without gaps or duplicates, also
// across reconnects. It is closed when the subscription ends; Err says why.
// Events are queued without limit, so a slow reader never blocks the
// connection (and with it replies to its own requests).
func (s *Subscription) Events() <-chan protocol.SessionEventRecord { return s.out }

// Err reports why Events was closed: nil after Close, ErrClosed or
// ErrConnectionLost when the client went away, or an *Error if the daemon
// rejected the resubscribe after a reconnect. Call it once Events is closed.
func (s *Subscription) Err() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.err
}

// Close unsubscribes. Events still queued are dropped.
func (s *Subscription) Close() error {
	c := s.c
	c.mu.Lock()
	if c.subs[s.session] == s {
		delete(c.subs, s.session)
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
		return protocol.SessionUnsubscribe{ID: id, Payload: protocol.SessionRef{Session: s.session}}
	})
	return err
}

// push is called by the reader goroutine for each session.events frame.
func (s *Subscription) push(rec protocol.SessionEventRecord) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ended || rec.Seq <= s.lastSeq {
		return // a resubscribe replays what we already have
	}
	s.lastSeq = rec.Seq
	if !s.ready {
		s.replay = append(s.replay, rec)
		return
	}
	s.queue = append(s.queue, rec)
	s.signal()
}

// skipThrough marks events up to seq as seen without delivering them.
func (s *Subscription) skipThrough(seq int64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.lastSeq = max(s.lastSeq, seq)
}

// markReady is called by the reader goroutine on session.subscribed: from the
// next frame on, events are live.
func (s *Subscription) markReady() {
	s.mu.Lock()
	s.ready = true
	s.mu.Unlock()
}

func (s *Subscription) isReady() (bool, int64) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.ready, s.lastSeq
}

// takeReplay hands the collected replay to Subscribe.
func (s *Subscription) takeReplay() []protocol.SessionEventRecord {
	s.mu.Lock()
	defer s.mu.Unlock()
	r := s.replay
	s.replay = nil
	return r
}

// finish ends the subscription once the queue is drained. The first call wins.
func (s *Subscription) finish(err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.ended {
		return
	}
	s.ended, s.err = true, err
	s.signal()
}

func (s *Subscription) signal() {
	select {
	case s.wake <- struct{}{}:
	default:
	}
}

// pump moves queued events to the Events channel.
func (s *Subscription) pump() {
	defer close(s.out)
	for {
		s.mu.Lock()
		batch, ended := s.queue, s.ended
		s.queue = nil
		s.mu.Unlock()
		for _, rec := range batch {
			select {
			case s.out <- rec:
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
