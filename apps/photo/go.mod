module neoworks.dev/suite/apps/photo

go 1.27.1

require (
	neoworks.dev/suite/protocol/go v0.0.0-00010101000000-000000000000
	neoworks.dev/suite/renderers/tui v0.0.0-00010101000000-000000000000
	neoworks.dev/suite/sdk-go v0.0.0-00010101000000-000000000000
)

require github.com/coder/websocket v1.8.15 // indirect

replace neoworks.dev/suite/protocol/go => ../../protocol/go

replace neoworks.dev/suite/sdk-go => ../../sdk-go

replace neoworks.dev/suite/renderers/tui => ../../renderers/tui
