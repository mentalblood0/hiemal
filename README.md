# 🌌 hiemal

[![tests](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml/badge.svg)](https://github.com/mentalblood0/hiemal/actions/workflows/tests.yml)

- no own syntax, parsing is deserialization
- static type checking without annotations
- tuples, union types, heterogenous fold
- match clause exhaustiveness checking
- pass-by-value, strings and containers are cheap to clone
- parallel execution
- lazy evaluation: constants, arrays/tuples, objects, sequence, map and filter
- pure user functions results caching
- numbers are arbitrary size rational
- typechecked runtime regex building

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
- match groups:
    string: I categorically deny having triskaidekaphobia.
    regex: \b\w{13}\b
- match groups:
    string: I categorically deny having triskaidekaphobia.
    regex:
      - word boundary
      - group:
          - repeat: [word character]
            exactly: 13
        name: horror
      - word boundary
```

<details>
  <summary>more complex regex example</summary>

```yaml
constants:
  log levels: [INFO, WARN, ERROR, DEBUG]
compute:
  map:
    - 2026-08-10 14:23:45.123  INFO  auth_service - User login successful [user_id=12345]
    - 2026-08-10 14:24:01.456  ERROR  database.core - Connection timeout (ERR-502) [retry_count=3]
    - 2026-08-10 14:25:10.789  WARN  cache.redis - Memory usage high [threshold=85%]
    - 2026-08-10 14:26:30.012  DEBUG  api.v2.handler - Processing request [endpoint=/users]
    - 2026-08-10 14:27:15.678  ERROR  auth.service - ERROR invalid token (ERR-401) [user_id=abc123]
  through:
    match groups:
      string: _
      regex:
        - start of string
        - name: timestamp
          group:
            - { repeat: [digit], exactly: 4 }
            - raw string: "-"
            - { repeat: [digit], exactly: 2 }
            - raw string: "-"
            - { repeat: [digit], exactly: 2 }
            - whitespace character
            - { repeat: [digit], exactly: 2 }
            - raw string: ":"
            - { repeat: [digit], exactly: 2 }
            - raw string: ":"
            - { repeat: [digit], exactly: 2 }
            - raw string: .
            - { repeat: [digit], exactly: 3 }
        - { repeat: [whitespace character], min: 2 }
        - name: log_level
          group:
            - one of:
                map: { constant: log levels }
                through: [{ raw string: _ }]
        - { repeat: [whitespace character], min: 1 }
        - name: module
          group:
            - { repeat: [word character], min: 1 }
            - repeat:
                - character
                - { repeat: [word character], min: 1 }
              min: 0
        - { repeat: [whitespace character], min: 0 }
        - raw string: "-"
        - { repeat: [whitespace character], min: 0 }
        - name: message
          group:
            - { repeat: [{ character except from string: "([" }], min: 0 }
        - { repeat: [whitespace character], min: 0 }
        - repeat:
            - { repeat: [whitespace character], min: 1 }
            - raw string: (ERR-
            - name: error_code
              group:
                - { repeat: [digit], exactly: 3 }
            - raw string: )
          max: 1
        - { repeat: [whitespace character], min: 0 }
        - repeat:
            - raw string: "["
            - { repeat: [whitespace character], min: 0 }
            - name: extra_data_key
              group:
                - { repeat: [word character], min: 1 }
            - { repeat: [whitespace character], min: 0 }
            - raw string: =
            - { repeat: [whitespace character], min: 0 }
            - one of:
                map:
                  - [number, digit]
                  - [word, word character]
                  - [string, non-whitespace character]
                through:
                  - name:
                      concat:
                        - extra_data_value_
                        - { from: _, at: [0] }
                    group:
                      - repeat:
                          - { from: _, at: [1] }
                        min: 1
            - { repeat: [whitespace character], min: 0 }
            - raw string: "]"
          max: 1
        - end of string
```
</details>

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
functions:
  f:: _
compute:
  f:: 1
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

```yaml
map:
  match: { parse yaml: "[1]" }
  cases:
    - [number, [1, 2, 3]]
    - [{ array: number }, [s]]
    - [any, [1]]
through:
  sum:
    - 1
    - match: _
      cases:
        - [number, _]
        - [string, 0]
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

```yaml
filter:
  match: { parse yaml: "[1]" }
  cases:
    - [number, [1, 2, 3]]
    - [{ array: number }, [s]]
    - [any, [1]]
through:
  is sorted:
    - 0
    - match: _
      cases:
        - [number, _]
        - [string, 0]
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

```yaml
fold:
  match: { parse yaml: "lalala" }
  cases:
    - [number, [a, b, c]]
    - [any, [4, 5, 6]]
starting with: 0
through:
  sum:
    - constant: accumulator
    - match: { constant: current }
      as: _
      cases:
        - [number, _]
        - [string, 0]
```

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
