module github.com/Boriken-Dev/guatiao/examples/wasm_greeter_go

go 1.24

require github.com/Boriken-Dev/guatiao/crates/guatiao-wasm/guest-go v0.0.0

replace github.com/Boriken-Dev/guatiao/crates/guatiao-wasm/guest-go => ../../crates/guatiao-wasm/guest-go
