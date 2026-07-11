# 🌌 hiemal

[![tests](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml/badge.svg)](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml)

- no own syntax, parsing is deserialization
- static type checking without annotations
- tuples, union types, heterogenous fold
- match clause exhaustiveness checking
- pass-by-value, strings and containers are structurally shared
- parallel execution
- `sequence`, `map` and `filter` are lazily evaluated
- pure user functions results caching
- numbers are arbitrary size rational

## Installation

[Install Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html), then

```bash
cargo install --git https://github.com/mentalblood0/hiemal
```

## Usage

```bash
hiemal https://raw.githubusercontent.com/mentalblood0/hiemal/refs/heads/main/examples/tests.yml
hiemal examples/fibonacci_from_standard_input.yml
hiemal examples/include.yml
```

### Embedded functions

```yaml
- sum: [1, 2, 3]
- is sorted: [1, 2, 3]
- key-value pairs: {a: 1, b: 2, c: 3}
- flatten: [[1, a], [3], [4, b, 6]]
- standard input
- parse yaml: "[1, 2, string, 3, 4]"
```

### Clauses

#### from at

```yaml
functions:
  fibonacci::
    from: examples/fibonacci.yml
    at: [functions, { object key: "fibonacci:" }]
compute:
  fibonacci:: 10
```

```yaml
from:
  a: [1, 2, { b: 3 }]
at: [a, 2, b]
```

```yaml
from:
  a: [1, 2, { b: 3 }, 4]
at: [a, [1, { sum: [2, 1] }]]
```

#### constant

```yaml
constants:
  c: 1
compute:
  {constant: c}
```

```yaml
constants:
  c: 1
compute:
  _
```

#### functions constants compute

```yaml
functions:
  fibonacci::
    match: _
    cases:
      - [1, 1]
      - [2, 1]
      - - number
        - sum:
            - fibonacci::
                sum: [_, -1]
            - fibonacci::
                sum: [_, -2]
compute:
  fibonacci:: 24
```

```yaml
functions:
  f::
    sum:
      - constant: a
      - constant: b
      - constant: c
      - 4
constants:
  c: 3
compute:
  f::
    a: 1
    b: 2
```

```yaml
functions:
  apply::
    function:: { constant: argument }
compute:
  apply::
    function:: { sum: [_, 1] }
    argument: 1
```

#### match cases

```yaml
match: { parse yaml: "0x1A" }
cases:
  - [number, true]
  - [string, it's a string]
  - [any, it's something else]
```

```yaml
match: { is sorted: [1, 2, 3] }
cases:
  - [true, 1]
  - [false, string]
```

```yaml
match:
  match: { parse yaml: "[]" }
  as: _
  cases:
    - [number, _]
    - [string, _]
    - [any, null]
cases:
  - [number, it's a number]
  - [string, it's a string]
  - ["null", true]
```

```yaml
match: { parse yaml: "[1, 2, 3]" }
cases:
  - [number, it's a number]
  - [string, it's a string]
  - [[1, { sum: [1, 1] }, 3], true]
  - [any, it's something else]
```

#### starting with next

```yaml
from:
  starting with: 1
  next: { sum: [_, 1] }
at: [[0, 3]]
```

#### map as through

```yaml
map: [1, string, 2, [1, 2, 3]]
through:
  match: _
  as: matched
  cases:
    - [number, it's a number]
    - [string, it's a string]
    - - { array: number }
      - map: { constant: matched }
        as: element from matched
        through:
          sum: [{ constant: element from matched }, 1]
```

```yaml
map:
  key-value pairs: { first: 1, second: string, third: [1, 2, 3] }
through:
  match: _
  as: matched
  cases:
    - [{ tuple: [string, number] }, it's a number]
    - [{ tuple: [string, string] }, it's a string]
    - - { tuple: [string, { array: number }] }
      - map: { from: { constant: matched }, at: [1] }
        as: element from matched
        through:
          sum: [{ constant: element from matched }, 1]
```

#### filter as through

```yaml
from:
  filter:
    starting with: 1
    next: { sum: [_, 1] }
  through: { is sorted: [5, _] }
at: [[0, 3]]
```

#### fold as starting with accumulating in through

```yaml
fold: [1, 2, 3]
starting with: 0
through:
  sum: [{ constant: accumulator }, { constant: current }]
```

```yaml
sum:
  - fold: [1, 2, "string", 3, 4]
    as: current
    starting with: 0
    accumulating in: accumulator
    through:
      match: { constant: current }
      cases:
        - [number, sum: [{ constant: accumulator }, { constant: current }]]
        - [string, 0]
  - 1
```

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
