# Rust Coding Examples

## Struct with Methods

```rust
/// A rectangle defined by its width and height.
///
/// # Examples
///
/// ```
/// let rect = Rectangle::new(30, 50);
/// assert_eq!(rect.area(), 1500);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rectangle {
    /// The width of the rectangle in pixels.
    width: u32,
    /// The height of the rectangle in pixels.
    height: u32,
}

impl Rectangle {
    /// Creates a new rectangle with the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Returns the area of the rectangle.
    pub fn area(&self) -> u32 {
        self.width * self.height
    }

    /// Returns `true` if `self` can completely contain `other`.
    pub fn can_hold(&self, other: &Rectangle) -> bool {
        self.width >= other.width && self.height >= other.height
    }
}
```

## Trait Implementation

```rust
/// A trait for types that can produce a greeting.
pub trait Greet {
    /// Returns a greeting message.
    fn greet(&self) -> String;
}

/// Represents a person with a name.
#[derive(Debug, Clone)]
pub struct Person {
    /// The person's name.
    name: String,
}

impl Person {
    /// Creates a new person with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Greet for Person {
    /// Returns a personalized greeting.
    fn greet(&self) -> String {
        format!("Hello, my name is {}!", self.name)
    }
}
```

## Macro for Common Derives

```rust
use serde::{Deserialize, Serialize};

/// A macro that applies common derives to a struct.
///
/// Applies: Debug, Clone, Serialize, Deserialize, PartialEq, Eq
macro_rules! auto_derive {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($field_vis:vis $field:ident : $ty:ty),* $(,)?
        }
    ) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
        $(#[$meta])*
        $vis struct $name {
            $($field_vis $field: $ty),*
        }
    };
}

// Usage
auto_derive! {
    /// Configuration for connecting to a server.
    pub struct Server {
        /// Unique identifier for the server.
        pub id: u64,
        /// Human-readable name.
        pub name: String,
    }
}
```

## Builder Pattern

```rust
/// Configuration for an HTTP client.
#[derive(Debug, Clone)]
pub struct HttpClient {
    base_url: String,
    timeout_ms: u64,
    max_retries: u32,
    user_agent: Option<String>,
}

/// Builder for constructing an `HttpClient`.
#[derive(Debug, Default)]
pub struct HttpClientBuilder {
    base_url: Option<String>,
    timeout_ms: u64,
    max_retries: u32,
    user_agent: Option<String>,
}

impl HttpClientBuilder {
    /// Creates a new builder with default values.
    pub fn new() -> Self {
        Self {
            timeout_ms: 30_000,
            max_retries: 3,
            ..Default::default()
        }
    }

    /// Sets the base URL (required).
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Sets the request timeout in milliseconds.
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Sets the maximum number of retries.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Sets the User-Agent header.
    pub fn user_agent(mut self, agent: impl Into<String>) -> Self {
        self.user_agent = Some(agent.into());
        self
    }

    /// Builds the `HttpClient`, returning an error if required fields are missing.
    pub fn build(self) -> Result<HttpClient, &'static str> {
        let base_url = self.base_url.ok_or("base_url is required")?;

        Ok(HttpClient {
            base_url,
            timeout_ms: self.timeout_ms,
            max_retries: self.max_retries,
            user_agent: self.user_agent,
        })
    }
}
```

## Enum State Machine

```rust
/// Represents the lifecycle of a network connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Initial state, not yet connected.
    Disconnected,
    /// Attempting to establish connection.
    Connecting { attempt: u32 },
    /// Successfully connected.
    Connected { session_id: String },
    /// Connection failed.
    Failed { error: String },
}

impl ConnectionState {
    /// Returns `true` if the connection is active.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    /// Transitions to the connecting state.
    pub fn start_connecting(&mut self) {
        *self = Self::Connecting { attempt: 1 };
    }

    /// Transitions to connected with the given session ID.
    pub fn connect(&mut self, session_id: String) {
        *self = Self::Connected { session_id };
    }

    /// Transitions to failed state.
    pub fn fail(&mut self, error: String) {
        *self = Self::Failed { error };
    }
}
```
