---
name: ratatui-dev
description: Guide for building Rust TUI console applications with Ratatui framework. Use when creating terminal user interfaces, building dual-mode CLI/TUI tools, working with crossterm for terminal handling, implementing TEA (The Elm Architecture) in Rust, or developing productivity tools with keyboard-driven interfaces for macOS and Linux.
---

# Ratatui Development Guide

Build TUI applications using Ratatui + crossterm with The Elm Architecture (TEA).

## Quick Start

```rust
use std::io;
use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{prelude::*, widgets::{Block, Borders, Paragraph}};

fn main() -> Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();
    result
}

fn run(mut terminal: ratatui::DefaultTerminal) -> Result<()> {
    loop {
        terminal.draw(|frame| {
            frame.render_widget(
                Paragraph::new("Press q to quit")
                    .block(Block::default().borders(Borders::ALL)),
                frame.area(),
            );
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                break;
            }
        }
    }
    Ok(())
}
```

## Core Architecture

Use **TEA (The Elm Architecture)** with component-based UI:

```rust
// Model: Single source of truth
pub struct Model {
    items: Vec<Item>,
    selected: usize,
    mode: AppMode,
    running: bool,
}

// Message: All possible state changes
pub enum Message {
    Quit,
    SelectNext,
    SelectPrevious,
    Action(ItemId),
}

// Update: Pure function, no side effects
fn update(model: &mut Model, msg: Message) -> Option<Command> {
    match msg {
        Message::SelectNext => {
            model.selected = (model.selected + 1).min(model.items.len().saturating_sub(1));
            None
        }
        Message::Quit => { model.running = false; None }
        _ => None,
    }
}

// View: Pure rendering
fn view(model: &Model, frame: &mut Frame) {
    // Render widgets based on model state
}
```

## Project Structure (Dual CLI/TUI)

```
my-app/
├── Cargo.toml
├── src/
│   ├── main.rs           # Entry point with mode detection
│   ├── core/             # SHARED: No UI dependencies
│   │   ├── mod.rs
│   │   ├── domain.rs     # Domain models
│   │   ├── storage.rs    # Data persistence
│   │   └── error.rs      # Domain errors
│   ├── cli/              # CLI-specific
│   │   ├── mod.rs
│   │   └── commands.rs
│   └── tui/              # TUI-specific
│       ├── mod.rs
│       ├── app.rs        # State machine
│       ├── event.rs      # Event handling
│       └── ui/           # Widgets
```

## Entry Point Pattern

```rust
use clap::{Parser, Subcommand};
use std::io::IsTerminal;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    Add { title: String },
    List,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match cli.command {
        Some(cmd) => cli::run(cmd, cli.json).await,
        None if std::io::stdout().is_terminal() => tui::run().await,
        None => cli::list(cli.json).await,
    }
}
```

## Async Event Loop

```rust
use tokio::sync::mpsc;

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

        if app.should_quit() { break; }
    }
    Ok(())
}
```

## Critical Rules

1. **Always filter key events**: Windows sends Press AND Release
   ```rust
   if key.kind == KeyEventKind::Press { handle_key(key); }
   ```

2. **Install color-eyre BEFORE ratatui**:
   ```rust
   color_eyre::install()?;
   let terminal = ratatui::init();
   ```

3. **Render only when state changes** - avoid rendering on every tick

4. **Use saturating arithmetic** - prevent overflow panics:
   ```rust
   model.index = model.index.saturating_sub(1);
   ```

5. **Never block async runtime**:
   ```rust
   // BAD: std::fs::read_to_string("file")?;
   // GOOD: tokio::fs::read_to_string("file").await?;
   ```

6. **Separate UI from business logic** - return Actions, handle side effects in update

## Keyboard Conventions

| Action | Primary | Alt | From |
|--------|---------|-----|------|
| Down | `j` | `↓` | Vim |
| Up | `k` | `↑` | Vim |
| Confirm | `Enter` | `Space` | Universal |
| Cancel | `Esc` | `q` | Universal |
| Search | `/` | `Ctrl+f` | Vim |
| Help | `?` | `F1` | Vim |
| Quit | `q` | `Ctrl+c` | Terminal |

## References

- **Full guide with all patterns**: See [references/guide.md](references/guide.md)
- **Cargo.toml templates**: See [references/cargo-template.md](references/cargo-template.md)

## Checklist

### Setup
- [ ] ratatui + crossterm as foundation
- [ ] Structure with core/tui/cli separation
- [ ] Release profile: `strip`, `lto`, `opt-level = "z"`
- [ ] color-eyre before ratatui::init()

### State
- [ ] Single App/Model struct
- [ ] Message/Action enum for transitions
- [ ] Pure update function
- [ ] tokio channels for async

### UI
- [ ] Filter for `KeyEventKind::Press`
- [ ] 30 FPS frame rate (60 for animations)
- [ ] Render only on state change
- [ ] Context-sensitive help bar

### Testing
- [ ] TestBackend for UI tests
- [ ] Snapshot tests with insta
- [ ] CI on macOS and Linux
