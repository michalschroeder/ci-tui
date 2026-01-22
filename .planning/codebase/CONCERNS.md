# Codebase Concerns

**Analysis Date:** 2026-01-22

## Tech Debt

**Large monolithic app.rs state management:**
- Issue: `App` struct in `src/ui/app.rs` has grown to 1186 lines with 23 public fields tracking UI state, fix state, pre-command state, system stats, and more. This violates single responsibility and makes state transitions implicit.
- Files: `src/ui/app.rs` (lines 50-119)
- Impact: Difficult to reason about state changes, risk of inconsistent state when multiple operations overlap (e.g., fix_running + fix_all_running), hard to add new features without touching core state.
- Fix approach: Break App into focused substates (CheckRunnerState, FixState, UIState, SystemStatsState) or use a state machine pattern. The needs_redraw dirty flag is a workaround for lack of reactivity.

**Implicit event-to-state mapping:**
- Issue: `handle_runner_event()` in `src/ui/app.rs` (lines 454-501) has 13 event types but no exhaustiveness checking. RunnerEvent variants don't guarantee state updates are complete.
- Files: `src/ui/app.rs` (lines 454-501), `src/runner.rs` (lines 99-117)
- Impact: Easy to miss state updates when adding new event types. No validation that all events are handled.
- Fix approach: Use Rust's exhaustiveness checking by requiring match arms for all RunnerEvent variants. Consider builder pattern for result updates.

**Command string shell escaping complexity:**
- Issue: Multiple escape mechanisms in `src/runner.rs` (lines 282-307, 387-413) for Docker exec commands. Single quotes wrap bash, but values inside env flags are re-escaped with `replace('\'', "'\\''")`. This complexity is repeated 3 times with subtle variations.
- Files: `src/runner.rs` (lines 282-307, 387-413), `src/runner.rs` (lines 389-393)
- Impact: Risk of injection vulnerabilities if escaping is wrong. Difficult to maintain consistency. No centralized escaping logic.
- Fix approach: Extract a single `build_docker_exec_command()` function that handles all escaping. Test comprehensively with special characters (&, $, `, |, etc.).

**Copy-pasted pre-command init logic:**
- Issue: App initialization code for pre-commands is duplicated between `new()` and `reset_for_retry()` in `src/ui/app.rs` (lines 133-148 and 197-211).
- Files: `src/ui/app.rs` (lines 133-148, 197-211)
- Impact: Bug fixes must be made in two places. Risk of divergence between initialization paths.
- Fix approach: Extract `fn init_pre_commands()` helper method.

## Known Bugs

**Pre-command execution stops all checks on failure:**
- Symptoms: If a pre-command fails (e.g., DB migration), the entire check run is aborted and AllFinished is sent immediately
- Files: `src/runner.rs` (lines 185-192)
- Trigger: Any pre-command with success=false causes early return with AllFinished
- Workaround: Make pre-commands optional or implement a `continue_on_failure` flag in config. Currently no way to run checks even if pre-command fails.

**Memory usage percentages can exceed 100% in sparkline:**
- Symptoms: UI shows memory usage >100% in rare cases, skewing the sparkline display
- Files: `src/ui/app.rs` (lines 417-422) - calculation uses mem_used / mem_total
- Trigger: Swap or cgroup memory accounting edge cases where reported used > total
- Workaround: Cap mem_usage to 100.0 before storing in history
- Fix approach: Add `.min(100.0)` after mem_usage calculation at line 419

**Output scroll position not reset on filter changes:**
- Symptoms: When switching between failed-only and all filters, if output was scrolled down, new selection may show scrolled content
- Files: `src/ui/app.rs` (line 598 in toggle_failed_filter doesn't reset output_scroll)
- Trigger: User scrolls down in output, presses 'f' to filter, sees stale scroll position on new check
- Workaround: Manually press PageUp to reset scroll
- Fix approach: Add `self.output_scroll = 0;` in `toggle_failed_filter()` and `show_all()`

## Security Considerations

**Shell command injection risk in environment variable values:**
- Risk: User-provided env var values from config are shell-escaped with single quotes and backslash escape, but this escaping happens AFTER the string is already in the env var map. If an attacker controls config, they could embed bash with `''; command; '` patterns.
- Files: `src/runner.rs` (lines 283-287, 389-393)
- Current mitigation: Environment variables come from loaded YAML config file (not user input at runtime), so trust boundary is the config file. No injection possible if config is trusted.
- Recommendations:
  1. Document that config files should be version controlled and reviewed
  2. Consider using Docker's native env file syntax instead of manual escaping
  3. Add audit logging when config is loaded with sensitive env vars present

**Grep output parsing in test discovery:**
- Risk: `grep_search()` in `src/test_discovery.rs` (lines 104-116) pipes output from grep without validation. Malformed grep results could include paths with special characters that later cause command execution issues.
- Files: `src/test_discovery.rs` (lines 104-116)
- Current mitigation: Paths are only used in file existence checks, not executed as commands. Lower risk than command injection.
- Recommendations:
  1. Validate discovered test paths exist and are readable before including
  2. Add logging for which test discovery strategy succeeded
  3. Consider pattern length limits to prevent ReDoS in grep patterns

**Git reference validation missing:**
- Risk: `git diff` in `src/git.rs` (line 101) accepts arbitrary base_ref from config without validation. Could be used to pass git options like `--reverse` or inject commands.
- Files: `src/git.rs` (lines 99-123)
- Current mitigation: base_ref values come from config file, not user input. Git command is invoked safely via Command builder.
- Recommendations:
  1. Validate base_ref matches pattern `^[a-zA-Z0-9/_.-]+$` before using
  2. Document that only simple ref names are supported (no complex glob patterns)

## Performance Bottlenecks

**Regex compilation on every file pattern match:**
- Problem: `filter_by_pattern()` in `src/git.rs` (lines 28-39) compiles regex each time it's called. During check determination, this is called multiple times per check.
- Files: `src/git.rs` (lines 28-39), `src/checks.rs` (lines 104-108, 114-122)
- Cause: Config has pattern strings, not compiled regex. No caching layer.
- Improvement path: Pre-compile all file_patterns to regex in CiConfig.compiled_patterns (OnceLock). This is partially done for ignore_patterns (line 187-193 in config.rs) but not file_patterns.

**Test discovery grep scan for every source file:**
- Problem: `grep_search()` in `src/test_discovery.rs` (lines 86-120) runs grep command for each source file found. With 50 changed files and 2 strategies, this is 100 subprocess calls.
- Files: `src/test_discovery.rs` (lines 104-116)
- Cause: No batching or early exit - each file triggers independent grep
- Improvement path: Combine all patterns into single grep call with `-e` flags, or run grep once per directory with OR patterns

**UI rendering called even when needs_redraw=false:**
- Problem: `terminal.draw()` in `src/ui/mod.rs` (line 513) is only called when needs_redraw is true, which is good. But dashboard rendering in `src/ui/dashboard.rs` (781 lines) likely recomputes complex layouts even for unchanged data.
- Files: `src/ui/dashboard.rs` (entire file)
- Cause: No memoization of widget tree. Each frame rebuilds all widgets.
- Improvement path: Profile with `--release`. If CPU is high, implement incremental rendering or cache static widgets.

**System stats polling at 500ms may miss fast-changing peaks:**
- Problem: STATS_UPDATE_INTERVAL in `src/ui/mod.rs` (line 29) is 500ms, but Docker containers may have CPU spikes shorter than this window
- Files: `src/ui/mod.rs` (lines 28-29, 91-121)
- Cause: Balance between responsiveness and CPU overhead. sysinfo calls can be slow.
- Improvement path: If stats accuracy matters, increase frequency to 200ms. Monitor CPU cost with profiler. Current approach is reasonable for monitoring.

## Fragile Areas

**Pre-command state management with no rollback:**
- Files: `src/ui/app.rs` (lines 83-86), `src/runner.rs` (lines 166-194)
- Why fragile: Pre-commands are tracked in app.pre_commands vector with indices. If a pre-command fails and run aborts, the state is left partially updated (running pre-commands have Running status, subsequent ones are Pending). No cleanup or rollback.
- Safe modification: Document that pre-command failure is terminal. Add a "pre_command_failed" state field if recovery is needed.
- Test coverage: No tests for pre-command failure scenarios in `src/ui/app.rs` tests (test suite exists at lines 702-1186 but no pre-command tests)

**CheckToRun cloning in event handlers:**
- Files: `src/ui/mod.rs` (lines 234, 252, 270)
- Why fragile: Each key press handler clones the CheckToRun and spawns tokio task. CheckToRun contains all definition fields. If definition becomes large, cloning becomes expensive.
- Safe modification: Use Arc<CheckToRun> instead of cloning. Or pass only check_id through channels and look up definition when needed.
- Test coverage: No tests for memory usage or repeated operations

**Filter state doesn't interact with selected_check bounds:**
- Files: `src/ui/app.rs` (lines 592-605)
- Why fragile: `toggle_failed_filter()` calls `show_all()` which resets selected_check to 0. But filtered_checks() rebuilds the list each call. If filters are toggled rapidly, selected_check index could point to wrong item.
- Safe modification: Validate selected_check bounds after any filter change. Consider using struct {filter, selection} to keep them coupled.
- Test coverage: Test exists at lines 855-875 but only checks state, not navigation after filter toggle

## Scaling Limits

**Pre-command count grows linearly with groups:**
- Current capacity: Tested with ~5 groups, each with ~2 pre-commands = 10 pre-commands
- Limit: No hard limit, but UI may become slow rendering 100+ pre-command items in the selectable list. Each pre-command is a separate SelectableItem variant.
- Scaling path: Implement collapsible group headers or lazy rendering of pre-command list

**Check count impact on determine_checks():**
- Current capacity: Tested with ~50 checks in config
- Limit: determine_checks() uses nested loops over groups and checks (src/checks.rs lines 74-194). With 100+ checks, performance degrades. Each check triggers file pattern filtering.
- Scaling path: Use index-based grouping instead of iteration. Pre-compile pattern matching logic.

**Output buffer unbounded growth:**
- Current capacity: Typical check output is 10-100KB per check. Results stored in HashMap<String, CheckResult> in app.output field (String).
- Limit: With 100 checks each producing 1MB output, memory usage is ~100MB. No bounds on output accumulation.
- Scaling path: Implement circular buffer or streaming output sink. Consider truncating old output or writing to temp files.

**Event channel backpressure:**
- Current capacity: RUNNER_CHANNEL_CAPACITY is 100 (src/ui/mod.rs line 39)
- Limit: High-throughput checks producing lots of output can overflow the channel. When full, send() blocks runner task.
- Scaling path: Increase capacity dynamically based on available memory, or implement adaptive batching where runner waits before sending output events

## Dependencies at Risk

**sysinfo version 0.32 may have stale system info:**
- Risk: sysinfo can report cached CPU/memory stats that lag reality. No guarantee of accuracy.
- Impact: UI shows outdated stats. Low impact since stats are informational only.
- Migration plan: If accuracy becomes critical, use /proc/stat and /proc/meminfo directly on Linux (adds platform-specific code)

**crossterm event handling differences across terminals:**
- Risk: KeyEvent handling in spawn_keyboard_thread (src/ui/mod.rs lines 59-85) assumes consistent KeyEventKind::Press behavior. Some terminals may behave differently.
- Impact: Keyboard input may be doubled or missed on unsupported terminals
- Migration plan: Test on target terminals before release. Keep OSC 52 clipboard workaround (src/ui/mod.rs lines 159-165) as a reference for terminal-specific quirks

## Missing Critical Features

**No progress indication for long-running checks:**
- Problem: When a check is running, there's no indication of progress. Status changes from Pending to Running, but no estimated time remaining or percentage complete.
- Blocks: Users can't estimate how long to wait for results
- Solution: Add optional progress tracking in CheckResult (percentage, current_step, total_steps). Requires checks to emit progress events, or infer from output line count.

**No retry mechanism for transient failures:**
- Problem: If a check fails due to network timeout or resource contention, there's no automatic retry. User must manually press 'r' to retry.
- Blocks: CI reliability in flaky environments
- Solution: Add max_retries and retry_delay to CheckDefinition. Implement exponential backoff in runner.

**No build cache integration:**
- Problem: Each check run rebuilds Docker images even if nothing changed. No layer caching between runs.
- Blocks: Slow development cycle with Docker-based checks
- Solution: Use docker compose --build-cache-from or mount Docker socket to leverage host's image cache

## Test Coverage Gaps

**Pre-command execution path untested:**
- What's not tested: Pre-command failures, success flow, output capture, env var merging in pre-commands
- Files: `src/runner.rs` (lines 271-331)
- Risk: Changes to pre-command logic could break silently. No tests exist for pre-command execution.
- Priority: High - pre-commands are critical infrastructure for test databases

**Parallel execution race conditions untested:**
- What's not tested: Multiple checks running in parallel (src/runner.rs lines 230-261), potential race conditions in result collection
- Files: `src/runner.rs` (lines 230-261)
- Risk: Rare hangs or lost output under concurrent load
- Priority: Medium - parallelism is optional but should be robust

**KeyEvent handling edge cases untested:**
- What's not tested: Key sequences, rapid key presses, modifier combinations, keyboard thread shutdown
- Files: `src/ui/mod.rs` (lines 59-85, 193-360)
- Risk: UI responsiveness issues under rapid input
- Priority: Low - mostly cosmetic, but affects user experience

**Terminal resize handling untested:**
- What's not tested: SIGWINCH signal handling, widget re-layout on resize, scroll position preservation
- Files: `src/ui/mod.rs` (entire event loop), `src/ui/dashboard.rs` (rendering)
- Risk: UI corruption or panic on terminal resize
- Priority: Medium - common operation but no tests exist

---

*Concerns audit: 2026-01-22*
