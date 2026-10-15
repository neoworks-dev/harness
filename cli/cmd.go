package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"slices"
	"strings"
	"text/tabwriter"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
)

// cliEnv is everything the commands take from the process, so tests can call
// run without spawning one.
type cliEnv struct {
	Stdin          io.Reader
	Stdout, Stderr io.Writer
	Getenv         func(string) string
	UID            int
	Interrupt      <-chan os.Signal // SIGINT notifications
	Now            func() time.Time
}

const usage = `usage: suite <command> [flags]

commands:
  status                        show daemon info and session counts
  sessions                      list sessions, most recently active first
  attach <id|prefix>            follow a session and send it input
  attach --new [--cwd DIR] [--title T]
                                create a session and attach to it
  tools [query...]              list connectors and their tools; with a query,
                                only tools matching all words

flags (all commands): --socket PATH
attach flags: --app NAME (default cli), --thoughts
`

// run executes one CLI invocation and returns the process exit code.
func run(ctx context.Context, args []string, e cliEnv) int {
	if len(args) == 0 {
		fmt.Fprint(e.Stderr, usage)
		return 2
	}
	cmd, args := args[0], args[1:]
	switch cmd {
	case "status":
		return runStatus(ctx, args, e)
	case "sessions":
		return runSessions(ctx, args, e)
	case "attach":
		return runAttach(ctx, args, e)
	case "tools":
		return runTools(ctx, args, e)
	case "help", "-h", "--help":
		fmt.Fprint(e.Stdout, usage)
		return 0
	}
	fmt.Fprintf(e.Stderr, "suite: unknown command %q\n\n%s", cmd, usage)
	return 2
}

// commandFlags builds a flag set with the flags every command shares.
func commandFlags(name string, e cliEnv) (fs *flag.FlagSet, socket *string) {
	fs = flag.NewFlagSet("suite "+name, flag.ContinueOnError)
	fs.SetOutput(e.Stderr)
	socket = fs.String("socket", "", "daemon socket path (default $SUITE_SOCKET or the runtime dir)")
	return fs, socket
}

// parseFlags parses flags that may come before or after positional arguments
// and returns the positionals.
func parseFlags(fs *flag.FlagSet, args []string) ([]string, error) {
	var positional []string
	for {
		if err := fs.Parse(args); err != nil {
			return nil, err
		}
		if fs.NArg() == 0 {
			return positional, nil
		}
		positional = append(positional, fs.Arg(0))
		args = fs.Args()[1:]
	}
}

// connect dials the daemon, reporting failure to stderr. It returns the client
// and the socket path used.
func connect(ctx context.Context, socketFlag string, e cliEnv) (*sdk.Client, string, bool) {
	path := sdk.SocketPath(socketFlag, e.Getenv, e.UID)
	c, err := sdk.Dial(ctx, sdk.Options{Socket: path, Client: "cli"})
	switch {
	case errors.Is(err, sdk.ErrDaemonDown):
		fmt.Fprintf(e.Stderr, "suite: daemon not running (socket %s); start it with: bun run daemon\n", path)
	case err != nil:
		fmt.Fprintf(e.Stderr, "suite: cannot connect to daemon (socket %s): %v\n", path, err)
	default:
		return c, path, true
	}
	return nil, path, false
}

func runStatus(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("status", e)
	if _, err := parseFlags(fs, args); err != nil {
		return 2
	}
	c, path, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer c.Close()

	sessions, err := c.ListSessions(ctx)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: %v\n", err)
		return 1
	}
	running := 0
	for _, s := range sessions {
		if s.State == protocol.SessionStateRunning {
			running++
		}
	}
	w := c.Welcome()
	uptime := e.Now().Sub(time.UnixMilli(w.StartedAt))
	fmt.Fprintf(e.Stdout, "daemon:    pid %d, version %s\n", w.PID, w.DaemonVersion)
	fmt.Fprintf(e.Stdout, "uptime:    %s\n", formatDuration(uptime))
	fmt.Fprintf(e.Stdout, "protocol:  %d\n", w.Protocol)
	fmt.Fprintf(e.Stdout, "socket:    %s\n", path)
	fmt.Fprintf(e.Stdout, "sessions:  %d total, %d running\n", len(sessions), running)
	return 0
}

func runSessions(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("sessions", e)
	if _, err := parseFlags(fs, args); err != nil {
		return 2
	}
	c, _, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer c.Close()

	sessions, err := c.ListSessions(ctx)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: %v\n", err)
		return 1
	}
	if len(sessions) == 0 {
		fmt.Fprintln(e.Stdout, "no sessions")
		return 0
	}
	slices.SortStableFunc(sessions, func(a, b protocol.SessionInfo) int { return int(b.LastActivity - a.LastActivity) })

	tw := tabwriter.NewWriter(e.Stdout, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "ID\tSTATE\tQUEUED\tEVENTS\tLAST ACTIVITY\tTITLE")
	now := e.Now()
	for _, s := range sessions {
		label := s.Cwd
		if s.Title != nil && *s.Title != "" {
			label = *s.Title
		}
		fmt.Fprintf(tw, "%s\t%s\t%d\t%d\t%s\t%s\n", s.ID, s.State, s.Queued, s.LastSeq, relativeTime(now, time.UnixMilli(s.LastActivity)), label)
	}
	tw.Flush()
	return 0
}

// resolveSession finds the session whose id equals or uniquely starts with
// prefix.
func resolveSession(sessions []protocol.SessionInfo, prefix string) (protocol.SessionInfo, error) {
	var matches []protocol.SessionInfo
	for _, s := range sessions {
		if s.ID == prefix {
			return s, nil
		}
		if prefix != "" && strings.HasPrefix(s.ID, prefix) {
			matches = append(matches, s)
		}
	}
	switch len(matches) {
	case 0:
		return protocol.SessionInfo{}, fmt.Errorf("no session matches %q", prefix)
	case 1:
		return matches[0], nil
	}
	ids := make([]string, len(matches))
	for i, s := range matches {
		ids[i] = s.ID
	}
	return protocol.SessionInfo{}, fmt.Errorf("%q is ambiguous: matches %s", prefix, strings.Join(ids, ", "))
}

// relativeTime formats t as how long before now it was, e.g. "2m ago".
func relativeTime(now, t time.Time) string {
	d := now.Sub(t)
	switch {
	case d < 5*time.Second:
		return "just now"
	case d < time.Minute:
		return fmt.Sprintf("%ds ago", int(d.Seconds()))
	case d < time.Hour:
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh ago", int(d.Hours()))
	}
	return fmt.Sprintf("%dd ago", int(d.Hours()/24))
}

// formatDuration prints the two most significant units, e.g. "2h 5m".
func formatDuration(d time.Duration) string {
	d = max(d, 0).Round(time.Second)
	switch {
	case d < time.Minute:
		return fmt.Sprintf("%ds", int(d.Seconds()))
	case d < time.Hour:
		return fmt.Sprintf("%dm %ds", int(d.Minutes()), int(d.Seconds())%60)
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh %dm", int(d.Hours()), int(d.Minutes())%60)
	}
	return fmt.Sprintf("%dd %dh", int(d.Hours()/24), int(d.Hours())%24)
}
