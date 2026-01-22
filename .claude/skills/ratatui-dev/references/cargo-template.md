# Cargo.toml Templates

## Full-Featured TUI Application

```toml
[package]
name = "my-tui-app"
version = "0.1.0"
edition = "2021"
rust-version = "1.70"

[dependencies]
# Core TUI
ratatui = "0.30"
crossterm = { version = "0.28", features = ["event-stream"] }

# Async runtime
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"  # CancellationToken
futures = "0.3"

# CLI
clap = { version = "4", features = ["derive"] }
clap-verbosity-flag = "2"

# Error handling
color-eyre = "0.6"
thiserror = "1"

# Serialization and config
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Utilities
dirs = "5"                 # Platform directories
chrono = "0.4"             # Date/time
unicode-width = "0.1"      # Character width calculation
tracing = "0.1"            # Structured logging
tracing-subscriber = "0.3"

[dev-dependencies]
insta = { version = "1", features = ["yaml"] }
pretty_assertions = "1"

[profile.release]
strip = true
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
```

## Minimal TUI Application

```toml
[package]
name = "my-tui-app"
version = "0.1.0"
edition = "2021"

[dependencies]
ratatui = "0.30"
crossterm = "0.28"
color-eyre = "0.6"

[profile.release]
strip = true
opt-level = "z"
lto = true
```

## With Async Event Stream

```toml
[package]
name = "my-async-tui"
version = "0.1.0"
edition = "2021"

[dependencies]
ratatui = "0.30"
crossterm = { version = "0.28", features = ["event-stream"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time"] }
futures = "0.3"
color-eyre = "0.6"

[profile.release]
strip = true
lto = true
```

## Platform-Specific Dependencies

```toml
[package]
name = "cross-platform-tui"
version = "0.1.0"
edition = "2021"

[dependencies]
ratatui = "0.30"
crossterm = "0.28"
color-eyre = "0.6"
dirs = "5"

[target.'cfg(target_os = "linux")'.dependencies]
procfs = "0.14"

[target.'cfg(target_os = "macos")'.dependencies]
mach2 = "0.4"

[profile.release]
strip = true
lto = true
```

## Workspace Structure (Dual CLI/TUI)

```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.70"
license = "MIT"

[workspace.dependencies]
# Shared across workspace
thiserror = "1"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
```

### crates/core/Cargo.toml

```toml
[package]
name = "myapp-core"
version.workspace = true
edition.workspace = true

[dependencies]
thiserror.workspace = true
serde.workspace = true
tokio.workspace = true
```

### crates/tui/Cargo.toml

```toml
[package]
name = "myapp-tui"
version.workspace = true
edition.workspace = true

[dependencies]
myapp-core = { path = "../core" }
ratatui = "0.30"
crossterm = { version = "0.28", features = ["event-stream"] }
color-eyre = "0.6"
tokio.workspace = true
```

### crates/cli/Cargo.toml

```toml
[package]
name = "myapp-cli"
version.workspace = true
edition.workspace = true

[dependencies]
myapp-core = { path = "../core" }
clap = { version = "4", features = ["derive"] }
color-eyre = "0.6"
tokio.workspace = true
```

## Release Profile Options

```toml
# Maximum binary size reduction
[profile.release]
strip = true          # Strip symbols
opt-level = "z"       # Optimize for size
lto = true            # Link-time optimization
codegen-units = 1     # Single codegen unit
panic = "abort"       # Smaller panic handling

# Balanced (faster compile, good size)
[profile.release]
strip = true
opt-level = 2
lto = "thin"

# Debug-friendly release
[profile.release-with-debug]
inherits = "release"
debug = true
strip = false
```
