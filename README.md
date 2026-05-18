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
hiemal https://raw.githubusercontent.com/mentalblood0/hiemal/refs/heads/main/examples/fibonacci.yml
hiemal examples/include.yml
```

### Stages

- include clauses substitution
- type checking
- computation

### Basic types

- strings
- numbers, arbitrary size and full precision thanks to [dashu](https://github.com/cmpute/dashu), e.g. `1`, `2.3`, `"4"`, `"5.6"`, `"7/9"`
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

#### CONSTANT

```yaml
CONSTANT: x
```

Gets value of already defined constant with given name

There is also shortcut to get constant with name "_", which is default for one-argument function calls:

```yaml
_
```

#### WITH FUNCTIONS CONSTANTS COMPUTE

```yaml
WITH:
  FUNCTIONS: # may be omitted
    FACTORIAL:
      PRODUCT:
        SEQUENCE:
          from: 1
          to: _
          step: 1
  CONSTANTS: # may be omitted
    x:
      SUM: [2, 3]
COMPUTE:
  FACTORIAL:
    CONSTANT: x
```

`_` is where defined function argument will be located if it is not object

If function argument is object, it will be 'destructured':

```yaml
WITH:
  FUNCTIONS:
    f:
      SUM:
        - CONSTANT: x
        - CONSTANT: y
        - CONSTANT: z
        - 4
  CONSTANTS:
    z: 3
  COMPUTE:
    f:
      x: 1
      y: 2
```

Definition is computed when and each time it is needed in `COMPUTE`

Constant is computed once before `COMPUTE`

Both functions and constants become available only in `COMPUTE`

#### MAP AS THROUGH

```yaml
MAP: [1, 2, 3]
AS: x # may be omitted, defaulting to "_"
THROUGH:
  PRODUCT: [x, 2]
```

#### FILTER AS THROUGH

```yaml
FILTER: [1, 2, 3]
AS: x # may be omitted, defaulting to "_"
THROUGH:
  IS_SORTED:
    - CONSTANT: x
    - 2
```

#### FOLD AS STARTING_WITH ACCUMULATING_IN THROUGH

```yaml
FOLD: [1, 2, 3]
AS: cur # may be omitted, defaulting to "current"
STARTING_WITH: 0
ACCUMULATING_IN: acc # may be omitted, defaulting to "accumulator"
THROUGH:
  SUM:
    - CONSTANT: acc
    - PRODUCT:
      - CONSTANT: curr
      - CONSTANT: curr
```

#### IF THEN ELSE

```yaml
IF:
  IS_SORTED:
    [1, 3, 2]
THEN: 1
ELSE: 2
```

#### FROM AT

```yaml
FROM: {key: [a, b]}
AT: [key, 1]
```

#### TRY OR WITH_ERROR

```yaml
TRY:
  FROM: [a, b]
  AT: [2]
OR:
  CONSTANT: err
WITH_ERROR: err # may be omitted, defaulting to "error"
```

### Composability

Every value of some type `T` can be replaced with expression which computes to value of type `T`

### No metaprogramming

Because with it static type-checking would become nearly impossible

Computation done in one pass

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
