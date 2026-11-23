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

### Clauses

#### constant

```yaml
constants:
  c: 1
compute: { constant: c }
```

```yaml
functions:
  f: _
compute:
  - [f]: 1
  - [f]: { _: 1 }
```

#### functions constants allow forbid compute

```yaml
functions:
  f:
    [sum]:
      - constant: a
      - constant: b
      - constant: c
      - 4
constants:
  c: 3
compute:
  [f]:
    a: 1
    b: 2
```

```yaml
allow:
  - error
  - create file
  - write file
  - read file
  - remove file
compute:
  pipe:
    - [write bytes]:
        target:
          [bufferized writer]:
            path: test files.txt
            mode: create
        content: { bytes: 62 62 }
    - [read bytes]:
        source:
          [bufferized reader]: test files.txt
        amount: 1
    - - _
      - [remove file]: test files.txt
  as: _
```

```yaml
forbid:
  - remove file
compute:
  pipe:
    - [write bytes]:
        target:
          [bufferized writer]:
            path: test files.txt
            mode: create
        content: { bytes: 62 62 }
    - [read bytes]:
        source:
          [bufferized reader]: test files.txt
        amount: 1
  as: _
```

```yaml
allow:
  - read file
compute:
  [read bytes]:
    source:
      [bufferized reader]: test files.txt
    amount: 1
```

available capabilities:

- create file
- write file
- read file
- read filesystem
- remove file
- read standard input
- network
- sleep
- precision loss
- error

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
  - [[1, { [sum]: [1, 1] }, 3], true]
  - [any, it's something else]
```

#### starting with as next while

```yaml
[get]:
  from:
    starting with: 1
    next: { [sum]: [_, 1] }
  at: [0, 3]
```

```yaml
[get]:
  from:
    starting with: [0, 1]
    next:
      - [sum]:
          - from: _
            at: [0]
          - 1
      - match:
          from: _
          at: [1]
        cases:
          - [number, s]
          - [string, 1]
  at: [0, 4]
```

```yaml
starting with: 1
next: { [sum]: [_, 2] }
while: { [is sorted]: [_, 5] }
```

#### starting with as loop while

```yaml
starting with: 1
loop: { [sum]: [_, 2] }
while: { [is sorted]: [_, 5] }
```

#### map as through

```yaml
map: [1, string, 2, [1, 2, 3]]
through:
  match: _
  cases:
    - [number, it's a number]
    - [string, it's a string]
    - - { array: number }
      - map: _
        as: element from matched
        through:
          sum: [{ constant: element from matched }, 1]
```

```yaml
map:
  [key-value pairs]: { first: 1, second: string, third: [1, 2, 3] }
through:
  match: _
  cases:
    - [{ tuple: [string, number] }, it's a number]
    - [{ tuple: [string, string] }, it's a string]
    - - { tuple: [string, { array: number }] }
      - map: { from: _, at: [1] }
        as: element from matched
        through:
          [sum]: [{ constant: element from matched }, 1]
```

```yaml
map:
  match: { [parse yaml]: "[1]" }
  cases:
    - [number, [1, 2, 3]]
    - [{ array: number }, [s]]
    - [any, [1]]
through:
  [sum]:
    - 1
    - match: _
      cases:
        - [number, _]
        - [string, 0]
```

#### filter as through

```yaml
[get]:
  from:
    filter:
      starting with: 1
      next: { sum: [_, 1] }
    through: { [is sorted]: [5, _] }
  at: [0, 3]
```

```yaml
filter:
  match: { [parse yaml]: "[1]" }
  cases:
    - [number, [1, 2, 3]]
    - [{ array: number }, [s]]
    - [any, [1]]
through:
  [is sorted]:
    - 0
    - match: _
      cases:
        - [number, _]
        - [string, 0]
```

#### fold as accumulating in through

```yaml
fold: [1, 2, 3]
starting with: 0
through:
  [sum]: [{ constant: accumulator }, { constant: current }]
```

```yaml
fold: [1, 2, string, 3, 4]
starting with: 0
through:
  match: current
  cases:
    - [
        number,
        sum: [{ constant: accumulator }, { constant: current }],
      ]
    - [string, 0]
```

```yaml
constants:
  FizzBuzz:
    map:
      starting with: 1
      next: { [sum]: [_, 1] }
    through:
      match:
        [mod]: [_, 3]
      cases:
        - - 0
          - match:
              [mod]: [_, 5]
            cases:
              - [0, FizzBuzz]
              - [any, Fizz]
        - - any
          - match:
              [mod]: [_, 5]
            cases:
              - [0, Buzz]
              - [any, _]
compute:
  fold:
    [get]:
      from: { constant: FizzBuzz }
      at: [0, 20]
  through:
    match: { constant: current }
    as: _
    cases:
      - - number
        - { [sum]: [ { constant: accumulator }, _ ] }
      - [ any, { constant: accumulator } ]
  starting with: 0
```

#### pipe as

```yaml
pipe:
  - { [sum]: [1, 2] }
  - { [sum]: [_, 3] }
  - { [sum]: [_, 4] }
as: _
```

```yaml
from:
  pipe:
    - starting with: 0
      next: { [sum]: [_, 1] }
    - starting with: 0
      next: { [sum]: [_, { from: { constant: input }, at: [10], default: 0 }] }
    - starting with: 0
      next: { [sum]: [_, { from: { constant: input }, at: [10], default: 0 }] }
  as: input
at: [1]
```

#### try as or

```yaml
try:
  - [read bytes]:
      source:
        [bufferized reader]: tests.yml
      amount: all
  - [read bytes]:
      source:
        [bufferized reader]: nonexistent file
      amount: all
as: error
or: { constant: error }
```

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
