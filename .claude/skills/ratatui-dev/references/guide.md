# Ratatui Complete Development Guide

## Table of Contents

1. [Architecture Patterns](#architecture-patterns)
2. [State Management](#state-management)
3. [Async Integration](#async-integration)
4. [CLI Output Patterns](#cli-output-patterns)
5. [Error Handling](#error-handling)
6. [Testing](#testing)
7. [Performance](#performance)
8. [Platform Compatibility](#platform-compatibility)
9. [Distribution](#distribution)
10. [Anti-patterns](#anti-patterns)

---

## Architecture Patterns

### TEA (The Elm Architecture)

```rust
#[derive(Debug, Default)]
pub struct Model {
    tasks: Vec<Task>,
    selected_index: usize,
    mode: AppMode,
    running: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Tick,
    Quit,
    SelectNext,
    SelectPrevious,
    CreateTask(String),
    CompleteTask(TaskId),
}

// Pure update function - no side effects
fn update(model: &mut Model, msg: Message) -> Option<Command> {
    match msg {
        Message::SelectNext => {
            model.selected_index = (model.selected_index + 1)
                .min(model.tasks.len().saturating_sub(1));
            None
        }
        Message::Quit => {
            model.running = false;
            None
        }
        Message::CreateTask(title) => {
            Some(Command::SaveTask(Task::new(title)))
        }
        _ => None,
    }
}

// Pure view function - no state mutation
fn view(model: &Model, frame: &mut Frame) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());

    frame.render_stateful_widget(
        TaskList::new(&model.tasks),
        layout[0],
        &mut ListState::default().with_selected(Some(model.selected_index)),
    );
    frame.render_widget(StatusBar::new(model), layout[1]);
}
```

### Component Trait

```rust
pub trait Component {
    fn init(&mut self) -> Result<()> { Ok(()) }

    fn handle_key_events(&mut self, key: KeyEvent) -> Action {
        Action::Noop
    }

    fn update(&mut self, action: Action) -> Action {
        Action::Noop
    }

    fn render(&mut self, frame: &mut Frame, area: Rect);
}
```

---

## State Management

### Centralized State with Channels

```rust
use tokio::sync::mpsc;

pub struct App {
    state: AppState,
    action_tx: mpsc::UnboundedSender<Action>,
}

impl App {
    pub fn spawn_async_task<F>(&self, task: F)
    where
        F: Future<Output = Action> + Send + 'static,
    {
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            let result = task.await;
            let _ = tx.send(result);
        });
    }
}
```

### Background Task with Cancellation

```rust
use tokio_util::sync::CancellationToken;

impl App {
    pub fn start_cancellable_task(&mut self) {
        if let Some(token) = self.task_token.take() {
            token.cancel();
        }

        let token = CancellationToken::new();
        self.task_token = Some(token.clone());
        let tx = self.action_tx.clone();

        tokio::spawn(async move {
            let mut progress = 0;
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        let _ = tx.send(Action::TaskCancelled);
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        progress += 1;
                        let _ = tx.send(Action::Progress(progress));
                        if progress >= 100 {
                            let _ = tx.send(Action::TaskComplete);
                            break;
                        }
                    }
                }
            }
        });
    }
}
```

---

## Async Integration

### Event Loop with tokio::select!

```rust
pub async fn run(mut app: App, mut tui: Tui) -> Result<()> {
    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();

    loop {
        tui.draw(|frame| app.render(frame))?;

        tokio::select! {
            Some(event) = tui.events.next() => {
                let action = app.handle_event(event);
                action_tx.send(action)?;
            }
            Some(action) = action_rx.recv() => {
                app.update(action);
            }
        }

        if app.should_quit() {
            break;
        }
    }
    Ok(())
}
```

### CPU-Intensive Work

```rust
Action::HeavyComputation => {
    let tx = app.action_tx.clone();
    let data = app.data.clone();

    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            expensive_calculation(&data)
        }).await.unwrap();

        tx.send(Action::ComputationResult(result)).unwrap();
    });
}
```

---

## CLI Output Patterns

### Verbosity Handling

```rust
use clap_verbosity_flag::{Verbosity, WarnLevel};

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    verbose: Verbosity<WarnLevel>,

    #[arg(long)]
    json: bool,

    #[arg(long)]
    quiet: bool,
}

pub fn output_result<T: Serialize + Display>(data: &T, json: bool) {
    if json {
        println!("{}", serde_json::to_string(data).unwrap());
    } else if std::io::stdout().is_terminal() {
        println!("{}", data.to_string().green());
    } else {
        println!("{}", data);
    }
}
```

### stdout/stderr Guidelines

| Content | Stream | Rationale |
|---------|--------|-----------|
| Primary results | stdout | Pipeable to other tools |
| Errors | stderr | Visible when stdout redirected |
| Progress/spinners | stderr | Doesn't pollute pipeable output |
| Debug/log messages | stderr | Clean stdout for scripts |

---

## Error Handling

### Setup Pattern

```rust
fn main() -> color_eyre::Result<()> {
    // Install color-eyre BEFORE ratatui
    color_eyre::install()?;

    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();

    result
}
```

### Custom Error Types

```rust
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Task not found: {0}")]
    TaskNotFound(TaskId),

    #[error("Storage error: {0}")]
    Storage(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),
}
```

### Actionable Messages

```rust
// BAD: Rust-specific terminology
"thread 'main' panicked at 'called `Option::unwrap()` on a `None` value'"

// GOOD: Actionable message
"Configuration file not found at ~/.config/devtool/config.toml
Run 'devtool init' to create a default configuration, or specify a path with --config"
```

---

## Testing

### Unit Tests with TestBackend

```rust
#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_app_renders_correctly() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::default();

        terminal.draw(|f| app.render(f)).unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer.get(0, 0).symbol() != " ");
    }

    #[test]
    fn test_key_handling() {
        let mut app = App::default();

        app.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.selected_index, 1);

        app.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.running);
    }
}
```

### Snapshot Testing

```rust
#[test]
fn test_ui_snapshot() {
    let app = App::with_test_data();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    insta::assert_snapshot!(terminal.backend());
}
```

---

## Performance

### Frame Rate Control

```rust
pub struct Tui {
    frame_rate: f64,  // 30.0 typical, 60.0 for animations
    tick_rate: f64,   // 1.0-4.0 for background updates
}

// Only render when state changes
loop {
    let needs_render = match event {
        Event::Key(_) | Event::Resize(_, _) | Event::Render => true,
        Event::Tick => app.has_pending_updates(),
        _ => false,
    };

    if needs_render {
        terminal.draw(|f| app.render(f))?;
    }
}
```

---

## Platform Compatibility

### Terminal Emulator Support

| Terminal | Platform | True Color | Unicode |
|----------|----------|------------|---------|
| iTerm2 | macOS | Yes | Yes |
| Terminal.app | macOS | Yes | Yes |
| Alacritty | Both | Yes | Yes |
| Kitty | Both | Yes | Yes |
| GNOME Terminal | Linux | Yes | Yes |

### Platform-Specific Paths

```rust
pub fn config_path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("No home directory"))?
            .join("Library/Application Support/myapp/config.toml"))
    }

    #[cfg(target_os = "linux")]
    {
        Ok(dirs::config_dir()
            .ok_or_else(|| anyhow!("No config directory"))?
            .join("myapp/config.toml"))
    }
}
```

---

## Distribution

### macOS Universal Binary

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin

cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin

lipo -create -output devtool-universal \
  target/x86_64-apple-darwin/release/devtool \
  target/aarch64-apple-darwin/release/devtool
```

### Linux Static Binary

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

### CI/CD Workflow

```yaml
name: Release
on:
  push:
    tags: ["[0-9]+.[0-9]+.[0-9]+"]

jobs:
  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-musl
            artifact: app-linux-x64
          - os: macos-latest
            target: x86_64-apple-darwin
            artifact: app-macos-x64
          - os: macos-latest
            target: aarch64-apple-darwin
            artifact: app-macos-arm64

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release --target ${{ matrix.target }}
      - run: |
          cd target/${{ matrix.target }}/release
          tar czvf ../../../${{ matrix.artifact }}.tar.gz myapp
      - uses: softprops/action-gh-release@v1
        with:
          files: ${{ matrix.artifact }}.tar.gz
```

---

## Anti-patterns

### Mixing UI and Business Logic

```rust
// BAD: Database operation in key handler
fn handle_key(&mut self, key: KeyEvent) {
    if key.code == KeyCode::Enter {
        self.db.insert_task(&self.input)?;  // Side effect!
    }
}

// GOOD: Return action
fn handle_key(&mut self, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Enter => Action::CreateTask(self.input.clone()),
        _ => Action::Noop,
    }
}
```

### Blocking Async Runtime

```rust
// BAD
async fn load_data(&self) {
    let data = std::fs::read_to_string("data.json")?;  // BLOCKS!
}

// GOOD
async fn load_data(&self) {
    let data = tokio::fs::read_to_string("data.json").await?;
}
```

### Rendering Too Frequently

```rust
// BAD: Renders on every tick
Event::Tick => {
    terminal.draw(|f| app.render(f))?;
}

// GOOD: Render only on change
Event::Tick => {
    if app.needs_redraw {
        terminal.draw(|f| app.render(f))?;
        app.needs_redraw = false;
    }
}
```

### Ignoring Platform Differences

```rust
// BAD: Handles all key events
if let Event::Key(key) = event {
    handle_key(key);  // Windows fires Press AND Release!
}

// GOOD: Filter for Press only
if let Event::Key(key) = event {
    if key.kind == KeyEventKind::Press {
        handle_key(key);
    }
}
```

### Holding Locks Across Await

```rust
// BAD
async fn update(&self) {
    let mut state = self.state.lock().await;
    self.fetch_data().await;  // Lock still held!
    state.data = new_data;
}

// GOOD
async fn update(&self) {
    let new_data = self.fetch_data().await;
    let mut state = self.state.lock().await;
    state.data = new_data;
}
```
