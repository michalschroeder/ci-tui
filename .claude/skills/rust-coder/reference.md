# Rust Coding Reference

## Structs

Custom data types with named fields. Always include `///` documentation:

```rust
/// A user account in the system.
pub struct User {
    /// Unique identifier.
    pub id: u64,
    /// Display name.
    pub name: String,
    /// Email address (optional).
    pub email: Option<String>,
}
```

## Impl Blocks

Place immediately after the type. Order methods logically (CRUD order or grouped by purpose):

```rust
impl User {
    // Constructor
    pub fn new(id: u64, name: String) -> Self {
        Self { id, name, email: None }
    }

    // Getters
    pub fn id(&self) -> u64 { self.id }
    pub fn name(&self) -> &str { &self.name }

    // Setters
    pub fn set_email(&mut self, email: String) {
        self.email = Some(email);
    }

    // Domain logic
    pub fn has_email(&self) -> bool {
        self.email.is_some()
    }
}
```

## Traits

Define shared behavior across types:

```rust
pub trait Validate {
    type Error;

    fn validate(&self) -> Result<(), Self::Error>;
}

impl Validate for User {
    type Error = String;

    fn validate(&self) -> Result<(), Self::Error> {
        if self.name.is_empty() {
            return Err("Name cannot be empty".into());
        }
        Ok(())
    }
}
```

## Macros

Eliminate repetitive code or generate derives:

```rust
/// Generates From impls for newtype wrappers.
macro_rules! newtype {
    ($name:ident, $inner:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(pub $inner);

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

newtype!(UserId, u64);
newtype!(Email, String);
```

## Build Performance

### Linux: Use mold linker

`.cargo/config.toml`:
```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

### Compiler Caching

```bash
# Install
cargo install sccache

# Configure (add to shell profile)
export RUSTC_WRAPPER=sccache

# Check stats
sccache --show-stats
```

### Incremental Builds

```bash
# Fast check without codegen
cargo check

# Build only what changed
cargo build

# Run specific test
cargo test test_name

# Build in release mode
cargo build --release
```

## Common Patterns

### Result and Option chaining

```rust
fn get_user_email(db: &Database, id: u64) -> Result<String, Error> {
    db.find_user(id)?              // Early return on error
        .email                      // Option<String>
        .ok_or(Error::NoEmail)?    // Convert None to Error
}
```

### Iterator methods

```rust
let names: Vec<String> = users
    .iter()
    .filter(|u| u.is_active())
    .map(|u| u.name.clone())
    .collect();
```

### Pattern matching

```rust
match result {
    Ok(value) => process(value),
    Err(Error::NotFound) => default_value(),
    Err(e) => return Err(e),
}
```

## Cargo.toml Best Practices

```toml
[package]
name = "my-app"
version = "0.1.0"
edition = "2021"
rust-version = "1.70"

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
criterion = "0.5"

[profile.dev]
opt-level = 0

[profile.release]
opt-level = 3
lto = true
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rectangle_area() {
        let rect = Rectangle::new(10, 20);
        assert_eq!(rect.area(), 200);
    }

    #[test]
    fn test_rectangle_can_hold() {
        let large = Rectangle::new(10, 10);
        let small = Rectangle::new(5, 5);

        assert!(large.can_hold(&small));
        assert!(!small.can_hold(&large));
    }
}
```
