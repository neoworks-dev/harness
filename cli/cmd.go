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
	// StdoutTTY says whether Stdout is a terminal, which turns on color and
	// the live tail. Size returns its width and height in cells (zeros if
	// unknown); it may be nil.
	StdoutTTY bool
	Size      func() (cols, rows int)

	// Used by up, down and the terminal app launch; all optional.
	Executable  func() (string, error)                      // default os.Executable
	Environ     func() []string                             // default os.Environ
	LookPath    func(string) (string, error)                // default exec.LookPath
	Exec        func(path string, argv, env []string) error // default syscall.Exec
	Spawn       func(path string, argv, env []string) error // default: start detached (suite gui)
	UpTimeout   time.Duration                               // default 15s
	DownTimeout time.Duration                               // default 10s
}

const usage = `usage: suite [command] [flags]

With no command on a terminal, suite starts the daemon if needed and opens the
terminal app (suite-term).

commands:
  gui [--session ID | --new]    open the desktop app (suite-gui) on the agent's
                                session, a given one, or a new one in this
                                directory; the terminal stays free
  up                            make sure the daemon is running; start it in the
                                background if not (log: daemon.log in the data dir)
  down                          stop the daemon
  status                        show daemon info, session counts and where the agent is
  sessions                      list sessions, most recently active first
  attach [id|prefix]            follow a session and send it input; without an
                                id, the agent's current session
  attach --new [--cwd DIR] [--title T]
                                create a session and attach to it
  focus                         show where the agent is (session and app)
  move [app] [--session ID]     move the agent to an open app (default: home);
                                --session makes that session current
  tools [query...]              list connectors and their tools; with a query,
                                only tools matching all words

flags (all commands): --socket PATH
attach flags: --app NAME (default cli), --thoughts, --raw (print raw session
events instead of the rendered view)
while attached: /<key> presses the button with that key on the latest item
that has buttons; /help lists them
`

// run executes one CLI invocation and returns the process exit code.
func run(ctx context.Context, args []string, e cliEnv) int {
	if len(args) == 0 {
		if isTerminal(e.Stdin) && e.StdoutTTY {
			return runTerm(ctx, e)
		}
		fmt.Fprint(e.Stderr, usage)
		return 2
	}
	cmd, args := args[0], args[1:]
	switch cmd {
	case "gui":
		return runGui(ctx, args, e)
	case "up":
		return runUp(ctx, args, e)
	case "down":
		return runDown(ctx, args, e)
	case "status":
		return runStatus(ctx, args, e)
	case "sessions":
		return runSessions(ctx, args, e)
	case "attach":
		return runAttach(ctx, args, e)
	case "tools":
		return runTools(ctx, args, e)
	case "focus":
		return runFocus(ctx, args, e)
	case "move":
		return runMove(ctx, args, e)
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
		fmt.Fprintf(e.Stderr, "suite: daemon not running (socket %s); start it with: suite up\n", path)
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
	if focus, err := waitFocus(ctx, c); err == nil {
		fmt.Fprintf(e.Stdout, "agent:     %s\n", describeFocus(focus))
	}
	return 0
}

// focusWait bounds how long a command waits for the daemon's first agent.focus,
// which it sends right after the handshake.
const focusWait = 3 * time.Second

func waitFocus(ctx context.Context, c *sdk.Client) (protocol.AgentFocus, error) {
	ctx, cancel := context.WithTimeout(ctx, focusWait)
	defer cancel()
	return c.WaitFocus(ctx)
}

// describeFocus says where the agent is, e.g. "session s1, app editor".
func describeFocus(f protocol.AgentFocus) string {
	if f.App == nil {
		return fmt.Sprintf("session %s, no app", f.Session)
	}
	return fmt.Sprintf("session %s, app %s", f.Session, *f.App)
}

func runFocus(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("focus", e)
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	if len(positional) > 0 {
		fmt.Fprint(e.Stderr, "usage: suite focus\n")
		return 2
	}
	c, _, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer c.Close()

	focus, err := waitFocus(ctx, c)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: no agent.focus from the daemon: %v\n", err)
		return 1
	}
	fmt.Fprintln(e.Stdout, describeFocus(focus))
	return 0
}

func runMove(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("move", e)
	session := fs.String("session", "", "make this session (id or prefix) the current one")
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	if len(positional) > 1 {
		fmt.Fprint(e.Stderr, "usage: suite move [app] [--session ID]\n")
		return 2
	}
	c, _, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer c.Close()

	var move protocol.AgentMove
	if len(positional) == 1 {
		move.App = &positional[0]
	}
	if *session != "" {
		sessions, err := c.ListSessions(ctx)
		if err == nil {
			var s protocol.SessionInfo
			if s, err = resolveSession(sessions, *session); err == nil {
				move.Session = &s.ID
			}
		}
		if err != nil {
			fmt.Fprintf(e.Stderr, "suite: %v\n", err)
			return 1
		}
	}
	if err := c.MoveAgent(ctx, move); err != nil {
		fmt.Fprintf(e.Stderr, "suite: move: %v\n", err)
		return 1
	}
	// The daemon sends the initial focus right after the handshake and broadcasts the new one
	// before it acks, so the latest focus is the result.
	focus, _ := c.Focus()
	fmt.Fprintf(e.Stdout, "agent moved: %s\n", describeFocus(focus))
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
