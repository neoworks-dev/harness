// Package tui draws resolved view items (protocol/schemas/view.schema.json)
// as text for a terminal, following renderers/README.md.
//
// Everything is a pure function of the items, a theme and Options: no I/O, no
// terminal queries. Render and RenderItem return strings that a caller prints
// (and, for live items, prints again); Buttons lists the keys the buttons of
// an item show.
//
// Text from the daemon is untrusted (tool output, agent text): control
// characters are dropped before anything is drawn, so no item can emit escape
// sequences of its own. Widths are display widths: East Asian wide characters
// and emoji take two cells (Width).
package tui
