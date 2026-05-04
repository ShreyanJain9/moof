# Protocols, Traits, and the Inheritance Tree: Deep Implementation Plan

> **Goal: Execute a mathematically rigorous deduplication of the standard library using formal Protocols and Mixins.**

This document provides the exact Moof AST transformations, the collision resolution strategy for mixins, and the step-by-step refactor plan for the standard library.

## 1. Defining Protocols (The Macro Expansion)

We must define how `defprotocol` transforms into runnable code.

### Phase 1.A: The `defprotocol` AST Transformation
**Files:** `lib/early/06-control-macros.moof`
The parser reads:
```moof
(defprotocol Iterable
  (requires [iterator] [next: iter])
  (derives
    [map: f] ( ... )))
```
The macro `defprotocol` transforms this into:
```moof
(def Iterable
  [Protocol newWithRequires: (list ':iterator ':next:)
                    derives: {
                      ':map:' (fn (f) ...)
                    }])
```

### Phase 1.B: The `Protocol` Proto
**Files:** `lib/early/07-protocols.moof` (New File)
```moof
(defproto Protocol
  (slots requires derives)
  (handlers
    [newWithRequires: r derives: d]
      [self cloneWithSlots: {requires: r derives: d}]
    [validate: targetHandlers]
      [self.requires forEach: |req|
        (if (not [targetHandlers hasKey: req])
            (raise 'protocol-validation-failed {missing: req}))]))
```

## 2. Applying Protocols (Mixins)

Concrete types pull in derived methods at compile time.

### Phase 2.A: Modifying `defproto` Macro
**Files:** `lib/early/08-match-defn-proto.moof`
The parser reads:
```moof
(defproto Cons
  (mixins Iterable Sized)
  (handlers [iterator] ...))
```
The compiler's macro expansion logic for `defproto`:
```moof
;; Inside defproto macro
(let finalHandlers baseHandlers)
;; 1. Inject derived methods from mixins
[mixinsList forEach: |mixin|
  [[mixin derives] forEach: |selector method|
    ;; Collision resolution: Explicit definitions override mixins.
    ;; Later mixins in the list override earlier ones.
    (if (not [baseHandlers hasKey: selector])
        [finalHandlers put: selector value: method])]]

;; 2. Validate requirements against the final set
[mixinsList forEach: |mixin|
  [mixin validate: finalHandlers]]
```

## 3. The Standard Library Refactor Plan

This is a destructive refactor. Hundreds of lines of code will be deleted.

### Phase 3.A: Numerical Protocols
1. **Create:** `lib/stdlib/protocols-math.moof` containing `Equatable` (`=`, `!=`) and `Comparable` (`<`, `>`, `<=`, `>=`, `between:and:`).
2. **Refactor `lib/stdlib/integer.moof`:**
   - Add `(mixins Equatable Comparable)`.
   - **Delete:** `[!=]`, `[>]`, `[<=]`, `[>=]`.
3. **Refactor `lib/stdlib/float.moof`:**
   - Add `(mixins Equatable Comparable)`.
   - **Delete:** `[!=]`, `[>]`, `[<=]`, `[>=]`.
4. **Refactor `lib/stdlib/char.moof`:**
   - Add `(mixins Equatable Comparable)`.
   - **Delete:** `[!=]`, `[>]`, `[<=]`, `[>=]`.

### Phase 3.B: Collection Protocols
1. **Create:** `lib/stdlib/protocols-collections.moof` containing `Iterable` (`map:`, `filter:`, `reduce:with:`, `forEach:`) and `Sized` (`length`, `empty?`).
2. **Refactor `lib/stdlib/cons.moof`:**
   - Implement `[iterator]`, `[next:]`, `[done?:]`, `[length]`.
   - Add `(mixins Iterable Sized)`.
   - **Delete:** `map:`, `filter:`, `reduce:with:`, `empty?`, `forEach:`.
3. **Refactor `lib/stdlib/string.moof`:**
   - Implement primitives. Add mixins.
   - **Delete:** duplicated collection methods.
4. **Refactor `lib/stdlib/table.moof`:**
   - Implement primitives. Add mixins.
   - **Delete:** duplicated collection methods.

**Tests:** The entire test suite in `crates/substrate/tests/` should remain green without any modifications to the test files themselves, proving the semantic equivalence of the refactored code.
