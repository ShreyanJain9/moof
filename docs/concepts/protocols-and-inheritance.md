# Protocols, Traits, and the Inheritance Tree

> **Moving from scattered methods to a coherent, mathematically sound inheritance model.**

Historically, the Moof standard library (`lib/stdlib/`) grew organically. This resulted in duplicated logic across core types (e.g., `map:`, `filter:`, `reduce:` being re-implemented for `Cons`, `String`, and `Table`). To achieve maximum elegance, we must formalize the inheritance tree using **Protocols**.

## The Underspecced Tree

Currently, delegation is simple prototype inheritance (`proto` field). However, true moldability requires understanding *interfaces* (what a Form promises to do) independently of its concrete structure.

The current tree looks like a flat list inheriting from `Object`:
`Object` ← `Cons`, `Object` ← `String`, `Object` ← `Integer`.

## Introducing Protocols (Traits)

A Protocol in Moof is a Form that defines a set of required primitive methods and provides a set of derived methods. It acts as a Mixin or Trait.

### Example: The Iterable Protocol

Instead of writing `map:` three times, we define `Iterable`:

```moof
(defprotocol Iterable
  ;; Primitives that the concrete type MUST provide
  (requires [iterator] [next: iter] [done?: iter])

  ;; Methods that are automatically mixed in
  (derives
    [map: f]
      [self reduce: '() with: |acc x| [acc cons: (f x)] reverse]

    [filter: pred]
      [self reduce: '() with: |acc x| (if (pred x) [acc cons: x] acc) reverse]

    [reduce: init with: f]
      (let loop ((iter [self iterator]) (acc init))
        (if [self done?: iter]
            acc
            (loop [self next: iter] (f acc [iter value]))))))
```

### Mixin Application

When defining a concrete type, you declare its protocols. The Moof compiler will verify that the required primitives are implemented and will automatically inject the derived methods into the prototype's `handlers` table.

```moof
(defproto Cons
  (mixins Iterable Equatable Sized)
  (handlers
    ;; Only needs to implement the irreducible primitives
    [iterator] self
    [next: iter] [iter cdr]
    [done?: iter] [iter empty?]
    ;; map:, filter:, etc., come for free from Iterable
  ))
```

## Mathematical Protocols

This approach shines for numerical and logical operations.

- **Equatable:** Requires `[=]`. Derives `[!=]`.
- **Comparable:** Requires `[<]`. Derives `[>]`, `[<=]`, `[>=]`, `[between:and:]`.
- **Arithmetic:** Requires `[+]`, `[-]`, `[*]`, `[/]`. Derives `[abs]`, `[sign]`, `[squared]`.

## Agent-Assisted Refactoring

By defining the world strictly through Protocols, the system becomes far more amenable to automated reasoning. An agent can read the `requires` metadata of a Protocol Form and automatically generate stub methods, or analyze a Form to see if it implicitly fulfills a Protocol it hasn't officially declared.
