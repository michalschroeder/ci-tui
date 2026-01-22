---
name: rust-coder
description: >
  Guide for writing idiomatic, efficient, and well-structured Rust code.
  Use when: (1) User asks to write or refactor Rust code, (2) Creating new Rust
  structs, enums, or traits, (3) Implementing macros or optimizing builds,
  (4) Organizing Rust module structure. Covers data structure design, ownership
  patterns, documentation, macros, and build optimization.
---

# Rust Coding Skill

Write idiomatic, efficient, and well-structured Rust code following nine core principles.

## 1. Data Structure Design

Choose between `struct`, `enum`, or `newtype` based on domain requirements:

- **Ownership patterns**: Prefer `&str` over `String` when ownership isn't needed, slices over vectors for read-only access
- **Shared ownership**: Use `Arc<T>` for thread-safe shared ownership, `Rc<T>` for single-threaded
- **Interior mutability**: Use `RefCell<T>` or `Mutex<T>` when needed
- **Newtypes**: Wrap primitives to add type safety (`struct UserId(u64)`)

## 2. Implementation Organization

Place `impl` blocks immediately below the struct/enum they modify:

```rust
struct Foo { /* fields */ }

impl Foo {
    // Constructors first
    pub fn new() -> Self { /* ... */ }

    // Getters
    pub fn field(&self) -> &Type { /* ... */ }

    // Mutation methods
    pub fn set_field(&mut self, value: Type) { /* ... */ }

    // Domain logic
    pub fn process(&self) -> Result<(), Error> { /* ... */ }
}
```

Group related methods together with blank lines separating each logical group.

## 3. Documentation Standards

Use `///` for doc comments on public items:

```rust
/// Calculates the area of the rectangle.
///
/// # Examples
///
/// ```
/// let rect = Rectangle::new(10, 20);
/// assert_eq!(rect.area(), 200);
/// ```
pub fn area(&self) -> u32 { /* ... */ }
```

Use //! for module-level documentation at the top of files.

## 4. Code Quality Tools

Always run before committing:

```bash
cargo fmt                              # Format code
cargo clippy --all-targets --all-features  # Lint
cargo test                             # Run tests
cargo doc --no-deps                    # Build docs
```

## 5. Macro Usage

**Derive macros** - Reduce boilerplate:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Config { /* ... */ }

// With serde for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiResponse { /* ... */ }
```

**Declarative macros** - For repetitive patterns:

```rust
macro_rules! impl_display {
    ($($t:ty),*) => {
        $(
            impl std::fmt::Display for $t {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{:?}", self)
                }
            }
        )*
    };
}
```

## 6. Build Optimization

### Fast Linker (Linux)

Add to `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

### Compilation Caching

```bash
# Install sccache
cargo install sccache

# Set environment variable
export RUSTC_WRAPPER=sccache
```

### Development Workflow

- Use `cargo check` during iteration (faster than full build)
- Minimize dependencies in `Cargo.toml`
- Organize code into lightweight workspaces for large projects

## 7. Module Structure

Organize code reflecting ownership and domain boundaries:

```
src/
├── lib.rs           # Public API, re-exports
├── config/
│   ├── mod.rs       # Module root
│   └── settings.rs  # Implementation
├── domain/
│   ├── mod.rs
│   ├── models.rs
│   └── services.rs
└── infrastructure/
    ├── mod.rs
    └── database.rs
```

- Prefer `pub(crate)` over `pub` when possible
- Keep public APIs small and expressive
- Use `mod.rs` or `module_name.rs` consistently (not both)

## 8. Error Handling

Use `Result<T, E>` and the `?` operator:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Not found: {0}")]
    NotFound(String),
}

pub fn process() -> Result<Data, AppError> {
    let conn = get_connection()?;
    let data = fetch_data(&conn)?;
    Ok(data)
}
```

## 9. Document Design Decisions

For every design decision, explain the rationale and propose alternatives:

```rust
// Using a builder pattern here instead of a simple constructor because:
// 1. Many optional fields with sensible defaults
// 2. Clearer API for callers
// 3. Validation can happen at build() time
//
// Alternative considered: Default trait + struct update syntax
// Rejected because: No validation, less discoverable API
```

Consider:
- Builder patterns vs simple constructors
- Enum-based state machines vs multiple booleans
- Traits vs generics vs dynamic dispatch
