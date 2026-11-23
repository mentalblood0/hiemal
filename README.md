# 🌌 hiemal

[![tests](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml/badge.svg)](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml)

- no recursive functions
- no own syntax, parsing is deserialization
- no annotations, yet static type checking
- tuples, union types, literal strings
- polymorphic fold, sequence and loop
- match clause exhaustiveness checking
- capabilities restrictions checking
- pass-by-value, strings and containers are cheap to clone
- parallel execution
- lazy evaluation: constants, arrays/tuples, objects, sequence, flatten, map and filter
- numbers are arbitrary size rational
- typechecked runtime regex building

## Installation

[Install Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html), then

```bash
cargo install --git https://github.com/mentalblood0/hiemal
```

## Usage

```bash
hiemal examples/tests.yml
```

### Embedded functions

See [embedded functions specification](src/embedded_functions.yml)

### Examples

See [tests](examples/tests.yml)

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
