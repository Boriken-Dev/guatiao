module github.com/Boriken-Dev/guatiao/benchmarks/go

go 1.26

require (
	github.com/Boriken-Dev/guatiao/bindings/go/guatiao v0.0.0-alpha.1
	github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde v0.0.0-alpha.1
)

replace (
	github.com/Boriken-Dev/guatiao/bindings/go/guatiao => ../../bindings/go/guatiao
	github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde => ../../bindings/go/guatiao-serde
)
