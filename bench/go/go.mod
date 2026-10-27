module example.com/alien-baml-sdk/bench

go 1.24

require example.com/alien-baml-sdk/go v0.0.0

require (
	github.com/boundaryml/baml-go v0.23.0 // indirect
	google.golang.org/protobuf v1.36.6 // indirect
)

replace example.com/alien-baml-sdk/go => ../../sdk/go
