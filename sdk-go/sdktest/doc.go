// Package sdktest is an in-process fake of the suite daemon for tests. It
// speaks enough of the bus protocol to exercise clients: sessions with an
// event log, replay-then-live subscriptions, a scripted agent, connector
// presence and listing, and recording of what clients sent.
//
// Unless noted, configure the Daemon's exported fields before the first
// client connects.
package sdktest
