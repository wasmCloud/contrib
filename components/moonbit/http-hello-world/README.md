# Moonbit http-bello-world

This example was created following the [Developing Wasm component model in MoonBit with minimal output size](https://www.moonbitlang.com/blog/component-model) blog.

## Prerequisites

- [Rust toolchain](https://www.rust-lang.org/tools/install) to install prerequisites
- wit-deps
  - `cargo install wit-deps-cli`
- wasm-tools
  - `cargo install wasm-tools`
- wit-bindgen
  - `cargo install wit-bindgen-cli`
- [Moonbit CLI](https://www.moonbitlang.com/download/#moonbit-cli-tools)
- [wash CLI](https://wasmcloud.com/docs/installation)

## Run in wasmCloud

After the tutorial and running the Moonbit Wasm component in wasmtime, we created a `wasmcloud.toml` to encapsulate the build & adapt steps and a `wadm.yaml` for running the component in wasmCloud.

The `wadm.yaml` was created using [wit2wadm](https://github.com/brooksmtownsend/wit2wadm). You can install the `wit2wadm` plugin using `wash plugin install oci://ghcr.io/brooksmtownsend/wit2wadm:0.2.0`.

```bash
wash build
wash wit2wadm ./build/gen.wasm --name moonbit-http --description "HTTP hello world demo with a component written in Moonbit" > wadm.yaml
wash up -d
wash app deploy ./wadm.yaml
```

Then, once the application is in the `Deployed` status:

```bash
> curl localhost:8000
Hello, World%
```

## Size & Speed

As promised, the Moonbit component is tiny!

```bash
➜ ll build
Permissions Size User   Date Modified Name
.rw-r--r--   22k brooks  7 Aug 10:30  gen.wasm
```

Running 100 max instances of the moonbit component, we get impressive numbers for HTTP throughput. As Moonbit evolves, it will be interesting to see how viable the language is for creating performant components.

```bash
➜ hey -z 20s -c 100 http://localhost:8000

Summary:
  Total:        20.0071 secs
  Slowest:      0.0401 secs
  Fastest:      0.0009 secs
  Average:      0.0048 secs
  Requests/sec: 20901.9356


Response time histogram:
  0.001 [1]     |
  0.005 [238066]        |■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■
  0.009 [176898]        |■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■
  0.013 [2474]  |
  0.017 [384]   |
  0.021 [140]   |
  0.024 [95]    |
  0.028 [81]    |
  0.032 [47]    |
  0.036 [0]     |
  0.040 [1]     |


Latency distribution:
  10% in 0.0035 secs
  25% in 0.0040 secs
  50% in 0.0047 secs
  75% in 0.0054 secs
  90% in 0.0061 secs
  95% in 0.0067 secs
  99% in 0.0084 secs

Details (average, fastest, slowest):
  DNS+dialup:   0.0000 secs, 0.0009 secs, 0.0401 secs
  DNS-lookup:   0.0000 secs, 0.0000 secs, 0.0017 secs
  req write:    0.0000 secs, 0.0000 secs, 0.0009 secs
  resp wait:    0.0046 secs, 0.0007 secs, 0.0400 secs
  resp read:    0.0002 secs, 0.0000 secs, 0.0140 secs

Status code distribution:
  [200] 418187 responses
```
