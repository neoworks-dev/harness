package sdk

import (
	"context"
	"sync"
)

// notifier runs callbacks one at a time, in the order they were pushed, on its
// own goroutine. The queue is unbounded, so pushing never blocks the reader.
type notifier struct {
	mu    sync.Mutex
	queue []func()
	wake  chan struct{} // cap 1: the queue has something new
}

func newNotifier() *notifier {
	return &notifier{wake: make(chan struct{}, 1)}
}

func (n *notifier) push(f func()) {
	n.mu.Lock()
	n.queue = append(n.queue, f)
	n.mu.Unlock()
	select {
	case n.wake <- struct{}{}:
	default:
	}
}

// run delivers callbacks until ctx ends.
func (n *notifier) run(ctx context.Context) {
	for {
		n.mu.Lock()
		batch := n.queue
		n.queue = nil
		n.mu.Unlock()
		for _, f := range batch {
			f()
		}
		if len(batch) > 0 {
			continue
		}
		select {
		case <-n.wake:
		case <-ctx.Done():
			return
		}
	}
}
