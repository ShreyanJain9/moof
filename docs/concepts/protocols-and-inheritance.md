# Protocols, Traits, and the Inheritance Tree: Implementation Plan

> **Moving from scattered methods to a coherent, mathematically sound inheritance model.**

This document details the exact syntactic macros and compiler logic needed to implement Protocols (Traits/Mixins) and deduplicate the Moof standard library.

## 1. Defining Protocols

A Protocol is a Form that declares required primitives and derived methods.

### Phase 1.A: The `defprotocol` Macro
We will add `defprotocol` to `lib/early/06-control-macros.moof`.

**Syntax:**
```moof
(defprotocol Iterable
  (requires [iterator] [next: iter] [done?: iter])
  (derives
    [map: f] ( ... implementation ... )
    [filter: pred] ( ... implementation ... )))
```

**Macro Expansion:**
The Moof compiler expands `defprotocol` into a `Protocol` proto instantiation:
```moof
(def Iterable
  [Protocol newWithRequires: '(:iterator :next: :done?:)
                    derives: {
                      :map: (fn (f) ...)
                      :filter: (fn (pred) ...)
                    }])
```

## 2. Applying Protocols (Mixins)

Concrete types explicitly opt-in to protocols.

### Phase 2.A: Modifying `defproto`
We will update the `defproto` macro (in `lib/early/08-match-defn-proto.moof`) to accept a `mixins` clause.

**Syntax:**
```moof
(defproto Cons
  (mixins Iterable Sized)
  (slots head tail)
  (handlers
    [iterator] self
    [next: iter] [iter tail]
    [done?: iter] [iter is nil]))
```

**Macro Expansion & Compiler Injection:**
During macro expansion of `defproto`, the compiler executes the following logic:
1. **Validation:** For each protocol in `mixins`, iterate over `[protocol requires]`. Check if the `handlers` dictionary (or the `proto` parent chain) provides those selectors. If missing, raise a `CompileError`.
2. **Injection:** For each protocol in `mixins`, iterate over `[protocol derives]`. For every derived method, inject it into the `handlers` dictionary of the newly created Proto, *unless* the proto explicitly overrides it.

## 3. Deduplicating the Standard Library

Once the macro is in place, we execute a sweeping refactor of `lib/stdlib/`.

### Phase 3.A: Numerical Protocols
Create `lib/stdlib/protocols-math.moof`:
```moof
(defprotocol Equatable
  (requires [=])
  (derives [!=] (not [self = other])))

(defprotocol Comparable
  (requires [<] [=])
  (derives
    [>] (and (not [self = other]) (not [self < other]))
    [<=] (or [self < other] [self = other])
    [>=] (or [self > other] [self = other])
    [between: a and: b] (and [self >= a] [self <= b])))
```
Refactor `Integer`, `Float`, and `Char` to `(mixins Equatable Comparable)`.

### Phase 3.B: Collection Protocols
Create `lib/stdlib/protocols-collections.moof`:
```moof
(defprotocol Sized
  (requires [length])
  (derives [empty?] ([self length] = 0)))

;; Iterable as defined above
```
Refactor `Cons`, `String`, and `Table` to `(mixins Iterable Sized)`. Remove the manual implementations of `map:`, `filter:`, `reduce:`, and `empty?` from their respective files, instantly deleting hundreds of lines of duplicated code.

## 4. Agent-Assisted Trait Inference

Because Protocols are first-class Forms, an agent can introspect the system.
If `UserDefinedList` has `handlers` for `:iterator`, `:next:`, and `:done?:`, but lacks the `(mixins Iterable)` declaration, an agent can proactively invoke `[UserDefinedList injectProtocol: Iterable]`, automatically enriching the user's data structure with `map:` and `filter:` at runtime.
