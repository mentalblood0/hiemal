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
cargo install --git https://github.com/mentalblood0/hiemal
```

## Usage

```bash
hiemal examples/factorial.json
hiemal examples/fibonacci.yml
hiemal examples/include.yml
```

### Stages

- include clauses substitution
- type checking
- computation

### Basic types

- strings
- numbers, parsed as 64-bit floating point numbers
- booleans
- null
- objects, keys are strings
- arrays, homogeneous, e.g. elements of array must be of the same type

### Embedded functions

```yaml
- SUM: [1, 2, 3]
- PRODUCT: [1, 2, 3]
- LEN: abc
- SIZE: [1, 2, 3]
- GET_ELEMENT:
    from: [1, 2, 3]
    at: 2
- IS_SORTED: [1, 2, 3]
- ARE_EQUAL: [1, 2, 3]
- ARE_EQUAL: [a, a, a]
- ARE_EQUAL: [[1, 2], [1, 2], [1, 2]]
- CONCAT: [ab, cd, efg]
- SEQUENCE:
    from: 1
    to: 9
    step: 2
```

Embedding new functions is quite easy, see [here](src/embedded_functions.rs)

### Clauses

#### INCLUDE_FILE

```yaml
INCLUDE_FILE: examples/factorial.yml
```

Interpreter will insert contents of the file instead of this clause

#### INCLUDE_URL

```yaml
INCLUDE_URL: https://raw.githubusercontent.com/mentalblood0/hiemal/refs/heads/main/examples/factorial.yml
```

Interpreter will insert contents of downloaded file instead of this clause

#### WITH DEFINITIONS CONSTANTS COMPUTE

```yaml
WITH:
  DEFINITIONS:
    FACTORIAL:
      PRODUCT:
        SEQUENCE:
          from: 1
          to: _
          step: 1
  CONSTANTS:
    x:
      SUM: [2, 3]
COMPUTE:
  FACTORIAL: x
```

`_` is where defined function argument will be located if it is not object

If function argument is object, it will be 'destructured':

```yaml
WITH:
  DEFINITIONS: # may be omitted
    FACTORIAL:
      PRODUCT:
        SEQUENCE:
          from: 1
          to: a
          step: 1
  CONSTANTS: # may be omitted
    x:
      SUM: [2, 3]
COMPUTE:
  FACTORIAL:
    a: x
```

Definition is computed when and each time it is needed in `COMPUTE`

Constant is computed once before `COMPUTE`

Both definitions and constants become available only in `COMPUTE`, that's why

```yaml
WITH:
  CONSTANTS:
    x: 1
COMPUTE:
  WITH:
    DEFINITIONS:
      d: x
    CONSTANTS:
      x: 2
      c: x
  COMPUTE: [d, c]
```

computes to `[2.0, 1.0]`

#### MAP AS_ALIAS THROUGH

```yaml
MAP: [1, 2, 3]
AS_ALIAS: x # may be omitted, defaulting to "_"
THROUGH:
  PRODUCT: [x, 2]
```

#### FILTER AS_ALIAS THROUGH

```yaml
FILTER: [1, 2, 3]
AS_ALIAS: x # may be omitted, defaulting to "_"
THROUGH:
  IS_SORTED: [x, 2]
```

#### REDUCE AS_ALIAS STARTING_WITH ACCUMULATING_IN_ALIAS THROUGH

```yaml
REDUCE: [1, 2, 3]
AS_ALIAS: cur # may be omitted, defaulting to "current"
STARTING_WITH: 0
ACCUMULATING_IN_ALIAS: acc # may be omitted, defaulting to "accumulator"
THROUGH:
  SUM:
    - acc
    - PRODUCT: [curr, curr]
```

#### IF THEN ELSE

```yaml
IF:
  IS_SORTED:
    [1, 3, 2]
THEN: 1
ELSE: 2
```

### Composability

Every value of some type `T` can be replaced with expression which computes to value of type `T`

### No metaprogramming

Because with it static type-checking would become nearly impossible

Computation done in one pass

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
