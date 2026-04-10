# 🌌 hiemal

[![tests](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml/badge.svg)](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml)

Programming language which uses deserialization of abstract syntax tree as parsing

- functional
- infers and statically checks types
- effectively a configuration files preprocessor

Command line utility supports `YAML` and `JSON` through `serde`, yet library is fully format-agnostic as works with deserialized structures

## Installation

[Install Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html), then

```bash
cargo install --git cargo install --git https://github.com/mentalblood0/hiemal
```

## Usage

```bash
cat examples/factorial.yml | hiemal yaml
cat examples/factorial.json | hiemal json
```
