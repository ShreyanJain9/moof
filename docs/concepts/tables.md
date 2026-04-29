# tables

> **the universal collection. positional entries and keyed entries
> simultaneously, in one type. APL-flavored operations. distinct from
> Lists.**

a Table is what lua's table would be if it had been raised by APL —
a hybrid array+map where every operation is rank-polymorphic and
broadcasting is the default.

## the model

a Table has two simultaneously available content axes:

- **positional**: ordered, integer-indexed (0-based).
- **keyed**: arbitrary-keyed, unordered.

both can coexist:

```moof
#[1 2 3]                         ; positional only — "vector"
#['name => "ada" 'age => 30]     ; keyed only — "map" / "record"
#[1 2 3 'tag => 'urgent]         ; mixed — "lua-table"
#[]                              ; empty
```

internally, the substrate may pick a representation (dense vector,
hash table, hybrid) based on usage; semantics are the same.

## literal syntax

`#[ … ]` opens a Table. inside:

- bare expressions are positional entries, in order.
- `key => value` introduces a keyed entry.
- keys and values are arbitrary expressions.

```moof
#[1 2 3]
#['a => 1 'b => 2]
#[1 2 'tag => 'hot]
#[(some-key-fn) => "computed"]
#[nil => 'absent]
```

(`syntax/literals.md` for the full grammar.)

## access

```moof
[t at: 0]                        ; positional access
[t at: 'name]                    ; keyed access
[t at-or: 'missing default: 0]   ; keyed with default
[t length]                       ; positional count
[t size]                         ; total entries (positional + keyed)
[t keys]                         ; all keys (positional indices + keyed keys)
[t values]                       ; all values
[t empty?]
[t contains-key?: k]
```

## mutation (when not frozen)

```moof
[t at: 0 put: 99]                ; replace at position 0
[t at: 'name put: "alice"]       ; set keyed entry
[t push: 5]                      ; append positional
[t pop]                          ; remove last positional
[t remove-key: 'tag]
[t freeze]                       ; → returns immutable copy
[t thaw]                         ; → returns mutable copy
```

mutability default: **mutable**. an immutable variant exists via
`:freeze`. the substrate's journaling captures every mutation, so
time-travel works regardless of mutability choice
(`concepts/time-and-journal.md`).

## APL-flavored operations

this is where Tables shine. every binary numeric operation
broadcasts; every `:reduce`, `:scan`, `:outer` is well-defined.

### broadcasting

```moof
[#[1 2 3 4] + 10]                ; → #[11 12 13 14]
[#[1 2 3] + #[10 20 30]]         ; → #[11 22 33] (pairwise)
[#[1 2 3] * #[1 2 3]]            ; → #[1 4 9]
[#[1 2 3] = #[1 0 3]]            ; → #[#true #false #true]
```

### reductions and scans

```moof
[#[1 2 3 4] reduce: +]           ; → 10
[#[1 2 3 4] reduce: + from: 100] ; → 110
[#[1 2 3 4] scan: +]             ; → #[1 3 6 10]   running fold
[#[1 2 3 4] scan: max]           ; → #[1 2 3 4]    running max
```

### outer product

```moof
[#[1 2 3] outer: + with: #[10 20 30]]
;; → 2D table:
;; #[#[11 21 31] #[12 22 32] #[13 23 33]]
```

### shape ops

```moof
[mat reshape: #[2 3]]
[mat transpose]
[mat axis: 0 reduce: +]          ; column sums (axis 0 collapses)
[mat axis: 1 scan: *]            ; row scans
```

### rich indexing

```moof
[t at: 5]                        ; single index
[t at: #[1 3 5]]                 ; multi-pick — sub-table
[t at: |x| [x > 0]]              ; predicate-pick
[t slice: 2..5]                  ; range slice
[t @ #[1 _ 3]]                   ; APL-style _ = "all along this axis"
```

(`@` is a binary operator for fancy indexing, in the J/K tradition.
its right-hand side describes the index pattern.)

### functional ops, in Table dialect

```moof
[t map: |x| [x * 2]]
[t filter: |x| [x > 0]]
[t each: |x| (println x)]
[t each-with-index: |x i| ...]
```

## records as columnar tables

```moof
#['name => #["ada" "bob" "cy"]
  'age  => #[30 25 40]
  'role => #['admin 'user 'admin]]
```

each column is itself a Table. row-access is constructed by indexing
all columns at the same position. broadcasting through columns is
natural. the bones of a tiny dataframe, no library required. (datalog
queries see this shape natively — `concepts/queries.md`.)

## Tables vs Lists

|  | Table | List |
|---|---|---|
| structure | array + map hybrid | linked cons-cells |
| literal | `#[1 2 3]` | `'(1 2 3)` |
| empty | `#[]` | `()` = nil |
| role | rich data, arrays, records, relations | code-as-data, recursion-friendly sequences |
| ops | rank-polymorphic / APL-flavored | head/tail/cons/length |
| mutability | mutable by default | conceptually immutable |
| persistence | per-vat database storage | substrate-managed |

both implement `Iterable` and `Indexable` so generic code can work
across either, but they are *not* the same type. choose Table when
you mean rich data; choose List when you mean code-as-data or a true
linked sequence.

## protos implemented

`Table` implements:

- `Iterable` — `:next`, `:done?` (via `:as-data-source`)
- `Indexable` — `:at:`, `:at-put:`, `:length`
- `Sized` — `:size`, `:empty?`
- `Equatable` — `:=`, structural recursive
- `Hashable` — for use as keys in other tables
- `Showable` — `:to-string`, `:inspect`
- `Numeric-Broadcasting` — element-wise binary ops
- `DataSource` — Table-as-stream of entries
  (`concepts/data-sources.md`)

## inspirations

- the universal-table idea: lua (ierusalimschy et al., 2003 onward).
- rank-polymorphic ops: APL (iverson 1962), J (hui), K (whitney).
- broadcasting + axis-ops: numpy (lineage from APL).
- `:reduce:`, `:scan:`, `:outer:`: APL's `/`, `\`, `°.`.
- records-as-columnar-tables: clojure / R / pandas.
- the Table-as-data-source idea: clojure transducers (hickey ~2014).

## see also

- `concepts/lists.md` — the cons-cell sequence.
- `concepts/data-sources.md` — Tables as streams.
- `concepts/queries.md` — Tables as relations.
- `syntax/literals.md` — Table literal grammar.
