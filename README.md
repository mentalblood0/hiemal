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
- sum: [1, 2, 3]
- product: [1, 2, 3]
- len: abc
- size: [1, 2, 3]
- is sorted: [1, 2, 3]
- are equal: [1, 2, 3]
- are equal: [a, a, a]
- are equal: [[1, 2], [1, 2], [1, 2]]
- concat: [ab, cd, efg]
- sequence:
    from: 1
    to: 9
    step: 2
```

Embedding new functions is quite easy, see [here](src/embedded_functions.rs)

### Clauses

#### include

```yaml
- with:
    include: [examples/factorial.yml, with]
  compute:
    factorial: 5
- include: [https://raw.githubusercontent.com/mentalblood0/hiemal/refs/heads/main/examples/factorial.yml]
```

#### constant

```yaml
constant: x
```

Gets value of already defined constant with given name

There is also shortcut to get constant with name "_", which is default for one-argument function calls:

```yaml
_
```

#### with functions constants compute

```yaml
with:
  functions: # may be omitted
    factorial:
      product:
        sequence:
          from: 1
          to: _
          step: 1
  constants: # may be omitted
    x:
      sum: [2, 3]
compute:
  factorial:
    constant: x
```

`_` is where defined function argument will be located if it is not object

If function argument is object with more then one key, it will be 'destructured' and it's key-value pairs will be treated as constants names and compute bodies:

```yaml
with:
  functions:
    f:
      sum:
        - constant: x
        - constant: y
        - constant: z
        - 4
  constants:
    z: 3
  compute:
    f:
      x: 1
      y: 2
```

Function is computed when and each time it is needed in `COMPUTE`

Constant is computed once before `COMPUTE`

Both functions and constants become available only in `COMPUTE`

#### map as through

```yaml
map: [1, 2, 3]
as: x # may be omitted, defaulting to "_"
through:
  product:
    - constant: x
    - 2
```

#### filter as through

```yaml
filter: [1, 2, 3]
as: x # may be omitted, defaulting to "_"
through:
  is sorted:
    - constant: x
    - 2
```

#### fold as starting with accumulating in through

```yaml
fold: [1, 2, 3]
as: cur # may be omitted, defaulting to "current"
starting with: 0
accumulating in: acc # may be omitted, defaulting to "accumulator"
through:
  sum:
    - constant: acc
    - product:
      - constant: curr
      - constant: curr
```

#### if then else

```yaml
if:
  is sorted:
    [1, 3, 2]
then: 1
else: 2
```

#### from at

```yaml
from: {key: [a, b]}
at: [key, 1]
```

#### try or with error

```yaml
try:
  from: [a, b]
  at: [2]
or:
  constant: err
with error: err # may be omitted, defaulting to "error"
```

### Composability

Every value of some type `T` can be replaced with expression which computes to value of type `T`

### No metaprogramming

Because with it static type-checking would become nearly impossible

Computation done in one pass

## Name

Named after [Hiemal](https://hiemalambient.bandcamp.com/) Dark/Drone Ambient artist from France
