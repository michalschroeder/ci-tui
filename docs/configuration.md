# CI-TUI Configuration Reference

This document provides a comprehensive reference for configuring CI-TUI via YAML configuration files.

## Overview

CI-TUI uses YAML-based configuration files to define how CI checks are discovered, executed, and displayed. The configuration format is version 2, which supports advanced features like test discovery, parallel execution, and flexible Docker integration.

A configuration file defines:
- Docker container setup
- Git change detection parameters
- File pattern matching for triggers
- Check definitions with commands and execution rules
- Test discovery strategies for intelligent test selection

## Top-Level Structure

A minimal CI-TUI configuration has this structure:

```yaml
version: 2

docker:
  project_dir: .
  service: app

git:
  base_branch: main
  fallback_branch: HEAD~1

file_patterns: {}

checks: {}
```

## Docker Configuration

The `docker` section configures how CI-TUI interacts with Docker containers.

```yaml
docker:
  # Required: Working directory for docker commands
  project_dir: ./infrastructure

  # Optional: Default Docker service name (default: "app")
  service: php

  # Optional: Explicit container name for docker exec
  # If not set, derived from project_dir + service (e.g., "myproject-php-1")
  container: myproject-php-1

  # Optional: Docker image for standalone docker run
  # If not set, derived from container name
  image: php:8.2-cli

  # Optional: Volume mount for standalone docker run
  # Supports environment variable expansion: ${VAR_NAME}
  volume_mount: "${HOST_PWD}:/app"

  # Optional: Working directory inside container (default: "/app")
  work_dir: /build

  # Optional: Environment variables passed to all docker commands
  env:
    XDEBUG_MODE: "off"
    APP_ENV: "testing"
```

### Field Details

**`project_dir`** (required)
- Path used for `docker compose --project-directory`
- Can be absolute or relative to current directory
- Used to derive container name if not explicit

**`service`** (default: `"app"`)
- Default Docker service name from docker-compose.yml
- Used for container name derivation and as fallback when check doesn't specify service

**`container`** (optional)
- Explicit container name for `docker exec` commands
- If not set, derived using Docker Compose naming: `{project_dir_basename}-{service}-1`
- Example: project_dir="./backend", service="php" → container="backend-php-1"

**`image`** (optional)
- Docker image for standalone `docker run` when container isn't running
- If not set, derived from container name by removing `-1` suffix
- Use for self-hosting CI-TUI without docker-compose

**`volume_mount`** (optional)
- Volume mount specification for `docker run`: `"source:destination"`
- Supports environment variable expansion: `${HOST_PWD}`, `${HOME}`, etc.
- Example: `"${HOST_PWD}:/build"` expands HOST_PWD to absolute path

**`work_dir`** (optional, default: `"/app"`)
- Working directory inside the container where commands execute
- Overrides container default

**`env`** (optional)
- Key-value pairs of environment variables
- Passed to all docker exec/run commands
- Individual checks can add additional env vars

## Git Configuration

The `git` section controls change detection.

```yaml
git:
  # Primary branch to compare against for detecting changes
  base_branch: development

  # Fallback if base_branch remote doesn't exist
  fallback_branch: HEAD~1
```

CI-TUI detects changed files by comparing the working tree against the base branch. It tries:
1. `origin/{base_branch}` (e.g., origin/development)
2. `{base_branch}` (local branch)
3. `{fallback_branch}` (e.g., HEAD~1 for last commit)

**Use cases:**
- Feature branch workflow: `base_branch: main`, `fallback_branch: HEAD~1`
- Multiple environments: `base_branch: development`, `fallback_branch: staging`
- Local testing: Use `HEAD~1` to check only recent commit

## File Patterns

The `file_patterns` section defines named regex patterns for matching files. These patterns are referenced by check triggers and test discovery rules.

```yaml
file_patterns:
  # Simple pattern
  php:
    pattern: '\.php$'

  # Pattern with UI color
  php_src:
    pattern: '^src/.*\.php$'
    color: blue

  php_tests:
    pattern: '^tests/.*\.php$'
    color: green

  blade:
    pattern: '\.blade\.php$'
    color: magenta
```

**Pattern syntax:** Standard regex (Rust regex crate)

**Color support:** Optional `color` field affects UI display. Supported colors:
- `blue`, `green`, `yellow`, `red`, `magenta`, `cyan`, `white`

**Purpose:** File patterns serve two functions:
1. **Check triggers:** Determine which checks run when files change
2. **UI highlighting:** Color-code changed files in the interface

## Ignore Patterns

The `ignore_patterns` section excludes files from change detection.

```yaml
ignore_patterns:
  - '\.md$'                    # Markdown files
  - '^vendor/'                 # Vendor directory
  - '^node_modules/'           # Node modules
  - '^storage/'                # Laravel storage
  - '\.log$'                   # Log files
```

**Pattern syntax:** Standard regex

**Use cases:**
- Exclude dependencies (`vendor/`, `node_modules/`)
- Ignore generated files (`storage/`, `cache/`, `build/`)
- Skip documentation when it doesn't affect checks

## Checks Configuration

The `checks` section defines execution groups and their checks. Groups execute sequentially in YAML key order. Within a group, checks can run in parallel.

```yaml
checks:
  # Group 1: Fast linting (parallel)
  lint:
    name: Code Linting
    parallel: true
    stop_on_failure: false
    checks:
      php-syntax:
        name: PHP Syntax Check
        command: php -l {files}
        triggers:
          file_pattern: php

      php-cs-fixer:
        name: PHP CS Fixer
        command: vendor/bin/php-cs-fixer fix --dry-run --diff {files}
        fix_command: vendor/bin/php-cs-fixer fix {files}
        triggers:
          file_pattern: php

  # Group 2: Tests (sequential, after lint)
  tests:
    name: Test Suite
    parallel: false
    pre_commands:
      - name: Migrate database
        command: php artisan migrate --env=testing
        service: php
        exec: true
    checks:
      phpunit:
        name: PHPUnit
        command: vendor/bin/phpunit {files}
        service: php
        on_demand: false
        triggers:
          file_pattern: php_tests
          test_discovery:
            source_pattern: php_src
            strategies:
              - type: path_mapping
                rules:
                  - source: "src/{path}.php"
                    tests:
                      - "tests/Unit/{path}Test.php"
                      - "tests/Feature/{path}Test.php"
```

### Group Options

**`name`** (optional)
- Display name for the group
- Falls back to YAML key if not provided

**`parallel`** (default: `false`)
- If `true`, checks in this group run concurrently
- If `false`, checks run sequentially

**`stop_on_failure`** (default: `false`)
- If `true`, stops all execution if any check in this group fails
- Useful for critical validation that blocks further checks

**`pre_commands`** (optional)
- List of commands to run before checks in this group
- See [Pre-Commands](#pre-commands) section

**`checks`** (required)
- Map of check definitions (YAML key is check ID)

### Check Options

**`name`** (required)
- Display name in UI

**`command`** (required)
- Shell command to execute
- Supports `{files}` placeholder (see below)

**`service`** (optional)
- Docker service to run this check in
- Overrides group/global default

**`container`** (optional)
- Explicit container name
- Overrides service-based derivation

**`fix_command`** (optional)
- Command that automatically fixes issues
- Runs when CI-TUI is invoked with `--fix` flag
- Typically used for formatters and auto-fixable linters

**`triggers`** (optional)
- Defines when this check runs (see [Triggers](#triggers-configuration))
- If omitted, check always runs

**`on_demand`** (default: `false`)
- If `true`, check only runs when manually triggered with 't' key
- Useful for expensive checks when test discovery finds no specific tests

**`env`** (optional)
- Additional environment variables for this check
- Merged with global docker.env

### The `{files}` Placeholder

Commands can use `{files}` placeholder, which expands to space-separated list of changed files:

```yaml
command: php-cs-fixer fix --dry-run {files}
# Expands to: php-cs-fixer fix --dry-run src/Foo.php src/Bar.php
```

**Auto-skip behavior:** If a command contains `{files}` but no files match the trigger pattern, CI-TUI automatically skips the check to prevent accidentally running against entire codebase.

**Manual override:** Skipped checks remain visible in UI and can be manually triggered with 't' key.

## Triggers Configuration

Triggers determine when a check runs based on file changes.

```yaml
triggers:
  # Simple: run when any PHP file changes
  file_pattern: php

# Or with test discovery
triggers:
  # Run when PHP test files change
  file_pattern: php_tests
  # Also discover tests when source files change
  test_discovery:
    source_pattern: php_src
    strategies:
      - type: path_mapping
        rules:
          - source: "src/{path}.php"
            tests: ["tests/Unit/{path}Test.php"]
```

### Fields

**`file_pattern`** (optional)
- References a key from `file_patterns` section
- Check runs when any file matching this pattern changes

**`test_discovery`** (optional)
- Advanced test discovery configuration
- See [Test Discovery](#test-discovery) section

## Test Discovery

Test discovery automatically finds related test files when source code changes. This enables running only relevant tests instead of the full suite.

```yaml
triggers:
  file_pattern: php_tests
  test_discovery:
    source_pattern: php_src
    strategies:
      - type: path_mapping
        rules:
          - source: "src/Services/{path}.php"
            tests:
              - "tests/Unit/Services/{path}Test.php"
              - "tests/Feature/Services/{path}Test.php"

      - type: grep_search
        search_dirs:
          - tests/Unit
          - tests/Feature
        pattern: 'class.*{basename}Test'
```

### Configuration Fields

**`source_pattern`** (required)
- References a file_pattern key
- When files matching this pattern change, CI-TUI discovers related tests

**`strategies`** (required)
- List of discovery strategies (tried in order)
- Multiple strategies can be combined

### Strategy: path_mapping

Maps source file paths to test file paths using path templates.

```yaml
- type: path_mapping
  rules:
    - source: "src/{path}.php"
      tests:
        - "tests/Unit/{path}Test.php"
        - "tests/Feature/{path}Test.php"
```

**How it works:**
1. Match changed file against `source` pattern
2. Extract the `{path}` portion
3. Expand `{path}` in each `tests` template
4. Check if resulting test files exist

**Example:**
- Changed file: `src/Services/UserService.php`
- Extracted path: `Services/UserService`
- Test candidates:
  - `tests/Unit/Services/UserServiceTest.php`
  - `tests/Feature/Services/UserServiceTest.php`
- Result: Only existing files are added to check

**Multiple rules:** Rules are tried in order. First matching rule is used.

### Strategy: grep_search

Searches test files for content matching a pattern. Useful when test structure doesn't mirror source structure.

```yaml
- type: grep_search
  search_dirs:
    - tests/Unit
    - tests/Feature
  pattern: 'class\s+\w*{basename}Test'
```

**How it works:**
1. Extract placeholders from changed file
2. Expand placeholders in `pattern`
3. Search `search_dirs` recursively for files containing pattern
4. Return matching test files

**Available placeholders:**
- `{basename}` - Filename without extension (e.g., `UserService` from `UserService.php`)
- `{filename}` - Full filename (e.g., `UserService.php`)
- `{extension}` - File extension (e.g., `php`)
- `{dirname}` - Parent directory name (e.g., `Services`)
- `{path}` - Relative path without extension (e.g., `src/Services/UserService`)

**Example:**
- Changed file: `src/Services/UserService.php`
- Pattern: `class\s+\w*{basename}Test`
- Expanded: `class\s+\w*UserServiceTest`
- Finds: `tests/Feature/UserManagementTest.php` containing `class UserManagementUserServiceTest`

**Pattern syntax:** Standard regex (Rust regex crate)

### Combining Strategies

Strategies are tried in order. All matching tests from all strategies are combined:

```yaml
strategies:
  # First try direct path mapping
  - type: path_mapping
    rules:
      - source: "src/{path}.php"
        tests: ["tests/Unit/{path}Test.php"]

  # Then search for indirect references
  - type: grep_search
    search_dirs: [tests/Integration]
    pattern: 'use\s+.*{basename};'
```

This finds both:
1. Direct test files following naming convention
2. Integration tests that import the changed class

### Handling No Tests Found

When test discovery finds no related tests:
- If check has `on_demand: false` (default): Check is skipped but visible in UI
- If check has `on_demand: true`: Check is skipped and marked on-demand
- User can manually trigger with 't' key to run full test suite

## Pre-Commands

Pre-commands run before checks in a group. Useful for setup tasks like database initialization.

```yaml
checks:
  tests:
    name: Test Suite
    pre_commands:
      - name: Migrate test database
        command: php artisan migrate --env=testing
        service: php
        exec: true
        env:
          DB_CONNECTION: testing

      - name: Seed database
        command: php artisan db:seed --class=TestSeeder
        service: php
        exec: true
    checks:
      # ... test checks
```

### Pre-Command Fields

**`name`** (required)
- Display name for the command

**`command`** (required)
- Shell command to execute

**`service`** (optional)
- Docker service to run in
- Falls back to group/global default

**`container`** (optional)
- Explicit container name
- Overrides service-based derivation

**`exec`** (default: `false`)
- If `true`, uses `docker exec` (requires running container)
- If `false`, uses `docker run` (creates new container)

**`env`** (optional)
- Additional environment variables
- Merged with global docker.env

### Execution

Pre-commands run sequentially before any checks in the group:
1. All pre-commands execute in order
2. If any pre-command fails, group is skipped
3. If all succeed, checks execute according to group's parallel setting

## Execution Modes

CI-TUI supports three execution modes:

### Normal Mode (default)

```bash
ci-tui --config ci-tui.yaml
```

Full TUI interface with:
- Real-time check execution
- Interactive output viewing
- Hotkeys for filtering, retrying, expanding output
- Color-coded file display

### Simple Mode

```bash
ci-tui --config ci-tui.yaml --simple
```

Non-interactive mode for CI pipelines:
- Plain text output
- No TUI, suitable for CI logs
- Exit code 0 if all checks pass, 1 if any fail
- Useful in GitHub Actions, GitLab CI, etc.

### Fix Mode

```bash
ci-tui --config ci-tui.yaml --fix
```

Runs only `fix_command` entries:
- Executes auto-fixers (formatters, auto-fixable linters)
- Skips checks without fix_command
- Useful before committing to auto-format code
- Returns after fixes complete

**Workflow:**
```bash
# Auto-fix issues
ci-tui --config ci-tui.yaml --fix

# Validate all checks pass
ci-tui --config ci-tui.yaml --simple
```

## Best Practices

### Start Simple

Begin with basic file pattern triggers:

```yaml
checks:
  lint:
    parallel: true
    checks:
      phpstan:
        name: PHPStan
        command: vendor/bin/phpstan analyse {files}
        triggers:
          file_pattern: php
```

Add test discovery after validating basics work.

### Use File Patterns for Highlighting

Color-code files by purpose:

```yaml
file_patterns:
  source:
    pattern: '^src/'
    color: blue
  tests:
    pattern: '^tests/'
    color: green
  config:
    pattern: '^config/.*\.(php|yaml)$'
    color: yellow
```

Colors help quickly identify what changed.

### Group Related Checks

Run fast checks in parallel:

```yaml
checks:
  fast:
    name: Quick Checks
    parallel: true
    checks:
      syntax:
        # ...
      lint:
        # ...

  slow:
    name: Heavy Analysis
    parallel: false
    checks:
      phpstan:
        # ...
      tests:
        # ...
```

### Use on_demand for Expensive Checks

Mark comprehensive test runs as on-demand:

```yaml
checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit
        command: vendor/bin/phpunit {files}
        on_demand: false  # Run when specific tests found
        triggers:
          file_pattern: php_tests
          test_discovery:
            source_pattern: php_src
            strategies: [...]

      full-suite:
        name: Full Test Suite
        command: vendor/bin/phpunit
        on_demand: true  # Only run when manually triggered
```

When source changes but no specific tests are found, user can trigger full suite with 't' key.

### Leverage fix_command

Add fix commands for auto-fixable issues:

```yaml
checks:
  lint:
    checks:
      php-cs-fixer:
        name: PHP CS Fixer
        command: vendor/bin/php-cs-fixer fix --dry-run --diff {files}
        fix_command: vendor/bin/php-cs-fixer fix {files}
        triggers:
          file_pattern: php
```

Workflow:
```bash
# Fix formatting
ci-tui --fix

# Verify all checks pass
ci-tui --simple
```

## Complete Example

See `docs/examples/php-symfony.yaml` for a complete working configuration demonstrating:
- Multi-group execution (lint → analysis → tests)
- Parallel linting with fix commands (PHP CS Fixer)
- Pre-commands for Doctrine migrations and fixtures
- Advanced test discovery with both strategies
- Symfony-specific patterns (src/, bin/console, Twig templates)
- Proper use of file patterns and colors

## Schema Validation

CI-TUI validates configuration at load time using strict schema:
- Unknown fields are rejected (prevents typos)
- Required fields are enforced
- Type checking for all values

**Example validation errors:**

```
Error: unknown field `base_branc` at line 7 column 3
Expected one of: base_branch, fallback_branch
```

This catches configuration mistakes early before running checks.

## Version History

**Version 2** (current)
- Added: `docker.container`, `docker.image`, `docker.volume_mount`, `docker.work_dir`
- Added: `docker.env` and per-check `env`
- Added: Test discovery with path_mapping and grep_search
- Added: `{files}` placeholder auto-skip behavior
- Added: `on_demand` check flag
- Added: `fix_command` and `--fix` mode

**Version 1** (legacy, not documented)
- Basic docker-compose integration
- Simple file pattern triggers
