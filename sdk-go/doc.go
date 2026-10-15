// Package sdk is the Go client for the suite bus: connection, presence,
// sessions and connector listing. See README.md for an overview.
//
// A Client is safe for concurrent use. Every request is matched to its reply
// by id, so calls may overlap with each other and with event delivery.
package sdk
