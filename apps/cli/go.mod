module neoworks.dev/neo/cli

go 1.27.1

require (
	neoworks.dev/neo/protocol/go v0.0.0-00010101000000-000000000000
	neoworks.dev/neo/renderers/tui v0.0.0-00010101000000-000000000000
	neoworks.dev/neo/sdk-go v0.0.0-00010101000000-000000000000
)

require github.com/coder/websocket v1.8.15 // indirect

replace neoworks.dev/neo/protocol/go => ../../protocol/go

replace neoworks.dev/neo/sdk-go => ../../sdk-go

replace neoworks.dev/neo/renderers/tui => ../../renderers/tui
