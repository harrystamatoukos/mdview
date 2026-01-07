# Rust Principles

A compact guide for writing idiomatic, safe, and performant Rust code.

---

## Ownership & Memory

- **One owner per value.** When owner goes out of scope, value drops.
- **Prefer borrowing over ownership transfer.** Use `&T` for read access, `&mut T` for write.
- **Borrowing rules:** Either one mutable reference OR multiple immutable references. Never both.
- **Lifetimes ensure references outlive their data.** Trust the compiler; add annotations only when required.
- **Default to stack allocation.** Use `Box` for heap when needed. Use `Rc`/`Arc` only for shared ownership.
- **Prefer `Clone` over `Rc` for small types.** Cloning is often cheaper than reference counting overhead.

## Error Handling

- **Use `Result<T, E>` for recoverable errors.** Use `Option<T>` for optional values.
- **Never `unwrap()` in library code.** Use `?` operator for propagation.
- **`expect()` only with meaningful messages** that explain the invariant being asserted.
- **Libraries:** Use `thiserror` for typed, semantic error enums.
- **Applications:** Use `anyhow` for ergonomic error context and propagation.
- **Add context at boundaries:** `operation.context("while processing config")?`

## Naming Conventions

| Construct | Style | Example |
|-----------|-------|---------|
| Types, Traits | `UpperCamelCase` | `HttpClient`, `Iterator` |
| Functions, Variables | `snake_case` | `parse_input`, `user_count` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_BUFFER_SIZE` |
| Lifetimes | Short lowercase | `'a`, `'de` |

- **Conversions:** `as_` (cheap, borrowed), `to_` (expensive, owned), `into_` (consuming)
- **Getters:** No `get_` prefix. Use `fn name(&self)` not `fn get_name(&self)`.
- **Iterators:** `iter()`, `iter_mut()`, `into_iter()` for collections.
- **Fallible constructors:** `new()` for infallible, `try_new()` for fallible.

## API Design

- **Implement standard traits eagerly:** `Debug`, `Clone`, `Default`, `PartialEq`, `Eq`, `Hash`, `Send`, `Sync`
- **Use `From`/`Into` for conversions.** Implement `From`; get `Into` free.
- **Accept generics, return concrete types.** `fn process(input: impl AsRef<str>)` not `fn process(input: &str)`
- **Prefer `&str` over `&String`, `&[T]` over `&Vec<T>`.** Accept the most general type.
- **No out-parameters.** Return values directly or use tuples/structs.
- **Builders for complex construction.** `Config::builder().timeout(30).build()`
- **Seal traits** that shouldn't be implemented downstream.
- **Keep struct fields private.** Expose via methods for flexibility.

## Idiomatic Patterns

```rust
// Pattern matching over if-else chains
match status {
    Status::Ok => handle_ok(),
    Status::Error(e) => handle_error(e),
}

// if-let for single variants
if let Some(value) = optional {
    use_value(value);
}

// while-let for iteration
while let Some(item) = iter.next() {
    process(item);
}

// Combinators over manual unwrapping
let result = option
    .map(|x| x * 2)
    .filter(|x| *x > 10)
    .unwrap_or_default();

// Entry API for maps
map.entry(key).or_insert_with(Vec::new).push(value);

// Destructuring in function params
fn process(&(x, y): &(i32, i32)) -> i32 { x + y }
```

## Performance

- **Avoid allocations in hot paths.** Reuse buffers, use `&str` over `String`.
- **Use `VecDeque` for FIFO queues.** `Vec::remove(0)` is O(n).
- **Prefer iterators over index loops.** They optimize better and avoid bounds checks.
- **`collect()` into specific types:** `collect::<Vec<_>>()` lets compiler optimize capacity.
- **Use `Cow<str>` when ownership is conditional.** Avoids unnecessary clones.
- **Profile before optimizing.** Use `cargo flamegraph`, `perf`, or `samply`.

## Async/Await

- **Async is for I/O-bound work.** Use `rayon` for CPU-bound parallelism.
- **Don't block the async runtime.** Use `spawn_blocking` for blocking operations.
- **Prefer single-threaded runtime unless proven necessary.** Multi-threaded adds sync overhead.
- **Keep domain logic synchronous.** Isolate async to I/O boundaries.
- **Use async `Mutex` sparingly.** Often indicates design that needs rethinking.
- **Channels over shared state.** `mpsc` for message passing between tasks.

## Unsafe Code

- **Avoid unless absolutely necessary.** FFI, performance-critical intrinsics, hardware access.
- **Minimize scope.** Smallest possible `unsafe` block.
- **Wrap in safe abstractions.** Expose safe API, hide unsafe internals.
- **Document invariants exhaustively.** Every `unsafe` block needs a `// SAFETY:` comment.
- **Check null before dereferencing raw pointers.**
- **Verify with Miri:** `cargo +nightly miri test`

```rust
// SAFETY: pointer is non-null and properly aligned,
// and we have exclusive access to the data
unsafe {
    *ptr = value;
}
```

## Testing

```rust
// Unit tests: same file, private access
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser() {
        assert_eq!(parse("input"), expected);
    }
}
```

- **Integration tests:** `tests/` directory, public API only.
- **Use `#[should_panic]` for expected panics.** Add `expected = "message"` for precision.
- **Property-based testing:** `proptest` or `quickcheck` for invariants.
- **Snapshot testing:** `insta` for complex output verification.
- **Mocking:** `mockall` for trait-based mocks. Design for testability with traits.
- **Coverage:** `cargo tarpaulin`. Aim for meaningful coverage, not 100%.

## Documentation

- **Every public item gets a doc comment.** `///` for items, `//!` for modules.
- **First line is a summary.** Complete sentence, third person: "Returns the length."
- **Include examples that compile.** Use ```` ```rust ```` blocks.
- **Document panics, errors, and safety.** Sections: `# Panics`, `# Errors`, `# Safety`
- **Use `?` in doc examples**, not `unwrap()`.
- **Link to related items:** `` [`OtherType`] ``

```rust
/// Parses a string into a configuration.
///
/// # Errors
///
/// Returns `Err` if the string is not valid TOML.
///
/// # Examples
///
/// ```
/// let config = parse_config("key = 'value'")?;
/// assert_eq!(config.key, "value");
/// # Ok::<(), Error>(())
/// ```
pub fn parse_config(s: &str) -> Result<Config, Error> { ... }
```

## Project Structure

```
my_crate/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Library root, public API
│   ├── main.rs         # Binary entry (optional)
│   └── module/
│       ├── mod.rs      # Module root
│       └── submodule.rs
├── tests/              # Integration tests
├── benches/            # Benchmarks
└── examples/           # Runnable examples
```

- **One concept per module.** Split when files exceed ~500 lines.
- **Re-export public API from lib.rs.** Users shouldn't dig into submodules.
- **Feature flags for optional dependencies.** Keep default features minimal.

## Clippy Configuration

```toml
# Cargo.toml or .cargo/config.toml
[lints.clippy]
# Enforce
pedantic = "warn"
nursery = "warn"
unwrap_used = "warn"
expect_used = "warn"
dbg_macro = "warn"
todo = "warn"
print_stdout = "warn"

# Allow when justified
missing_errors_doc = "allow"  # If errors are obvious
module_name_repetitions = "allow"
```

Run: `cargo clippy --all-targets --all-features -- -D warnings`

## Dependencies

- **Audit regularly:** `cargo audit`, `cargo deny`
- **Pin versions in applications.** Use `Cargo.lock`.
- **Use version ranges in libraries.** `"1.0"` allows compatible updates.
- **Minimize dependency count.** Each dep is attack surface and compile time.
- **Prefer `std` over external crates** when functionality exists.

## Common Anti-Patterns

| Anti-Pattern | Better Approach |
|--------------|-----------------|
| `clone()` to satisfy borrow checker | Restructure code, use references |
| `Rc<RefCell<T>>` everywhere | Redesign ownership, use channels |
| Stringly-typed APIs | Newtypes, enums |
| `unwrap()` in production | `?`, `expect()` with context |
| `Box<dyn Error>` in libraries | Typed error enums |
| Getter/setter for every field | Direct field access or builder |
| `impl Trait` in return position for libraries | Named types for documentation |

---

## Sources

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/)
- [Clippy Lints](https://rust-lang.github.io/rust-clippy/master/)
- [The Rustonomicon](https://doc.rust-lang.org/nomicon/)
- [Rust for Rustaceans](https://rust-for-rustaceans.com/) by Jon Gjengset
- [Idiomatic Rust](https://github.com/mre/idiomatic-rust)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
