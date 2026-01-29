#!/usr/bin/env node
// Claude Code Statusline - Enhanced Edition
// Shows: [gsd update │] model │ branch │ [task │] dirname │ cost │ tokens │ lines │ context bar

const fs = require('fs');
const path = require('path');
const os = require('os');

// ANSI helpers
const dim = (s) => `\x1b[2m${s}\x1b[0m`;
const bold = (s) => `\x1b[1m${s}\x1b[0m`;
const yellow = (s) => `\x1b[33m${s}\x1b[0m`;
const green = (s) => `\x1b[32m${s}\x1b[0m`;
const red = (s) => `\x1b[31m${s}\x1b[0m`;
const cyan = (s) => `\x1b[36m${s}\x1b[0m`;
const orange = (s) => `\x1b[38;5;208m${s}\x1b[0m`;
const blink_red = (s) => `\x1b[5;31m${s}\x1b[0m`;
const dimCyan = (s) => `\x1b[2;36m${s}\x1b[0m`;

/**
 * Format a number with k/M suffixes for compact display.
 * 523 → "523", 4500 → "4.5k", 15000 → "15k", 1200000 → "1.2M"
 */
function formatCompact(n) {
  if (n == null || n <= 0) return '';
  if (n < 1000) return String(Math.round(n));
  if (n < 10000) return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
  if (n < 1000000) return Math.round(n / 1000) + 'k';
  return (n / 1000000).toFixed(1).replace(/\.0$/, '') + 'M';
}

/**
 * Read current git branch from .git/HEAD directly (no subprocess).
 * Supports worktrees (.git file with gitdir: indirection).
 * Returns '' on any failure.
 */
function getGitBranch(projectDir) {
  try {
    let gitPath = path.join(projectDir, '.git');
    const stat = fs.statSync(gitPath);

    // Worktree support: .git is a file containing "gitdir: <path>"
    if (stat.isFile()) {
      const content = fs.readFileSync(gitPath, 'utf8').trim();
      const match = content.match(/^gitdir:\s*(.+)$/);
      if (!match) return '';
      gitPath = path.resolve(projectDir, match[1]);
    }

    const headPath = path.join(gitPath, 'HEAD');
    const head = fs.readFileSync(headPath, 'utf8').trim();

    // Normal branch: "ref: refs/heads/feature/my-branch"
    if (head.startsWith('ref: refs/heads/')) {
      return head.slice('ref: refs/heads/'.length);
    }

    // Detached HEAD: return short hash
    if (/^[0-9a-f]{40}$/.test(head)) {
      return head.slice(0, 7);
    }

    return '';
  } catch {
    return '';
  }
}

/**
 * Format cost as $X.XX with color thresholds.
 */
function formatCost(totalCost) {
  if (totalCost == null || totalCost <= 0) return '';
  const formatted = '$' + totalCost.toFixed(2);
  if (totalCost < 1) return green(formatted);
  if (totalCost < 5) return yellow(formatted);
  if (totalCost < 10) return orange(formatted);
  return red(formatted);
}

/**
 * Build context bar with colored progress indicator.
 */
function buildContextBar(remaining) {
  if (remaining == null) return '';
  const rem = Math.round(remaining);
  const used = Math.max(0, Math.min(100, 100 - rem));
  const filled = Math.floor(used / 10);
  const bar = '\u2588'.repeat(filled) + '\u2591'.repeat(10 - filled);
  const label = `${bar} ${used}%`;

  if (used < 50) return green(label);
  if (used < 65) return yellow(label);
  if (used < 80) return orange(label);
  return blink_red(`\u{1F480} ${label}`);
}

// Read JSON from stdin
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => (input += chunk));
process.stdin.on('end', () => {
  try {
    const data = JSON.parse(input);
    const homeDir = os.homedir();

    // Extract data fields
    const model = data.model?.display_name || 'Claude';
    const dir = data.workspace?.current_dir || process.cwd();
    const projectDir = data.workspace?.project_dir || dir;
    const session = data.session_id || '';
    const remaining = data.context_window?.remaining_percentage;
    const totalCost = data.cost?.total_cost_usd;
    const inputTokens = data.context_window?.total_input_tokens;
    const outputTokens = data.context_window?.total_output_tokens;
    const linesAdded = data.cost?.total_lines_added;
    const linesRemoved = data.cost?.total_lines_removed;

    const segments = [];

    // GSD update available
    const cacheFile = path.join(homeDir, '.claude', 'cache', 'gsd-update-check.json');
    if (fs.existsSync(cacheFile)) {
      try {
        const cache = JSON.parse(fs.readFileSync(cacheFile, 'utf8'));
        if (cache.update_available) {
          segments.push(yellow('\u2B06 /gsd:update'));
        }
      } catch {}
    }

    // Model
    segments.push(dim(model));

    // Git branch
    const branch = getGitBranch(projectDir);
    if (branch) {
      segments.push(dimCyan(branch));
    }

    // Current task from todos
    if (session) {
      const todosDir = path.join(homeDir, '.claude', 'todos');
      if (fs.existsSync(todosDir)) {
        const files = fs
          .readdirSync(todosDir)
          .filter((f) => f.startsWith(session) && f.includes('-agent-') && f.endsWith('.json'))
          .map((f) => ({ name: f, mtime: fs.statSync(path.join(todosDir, f)).mtime }))
          .sort((a, b) => b.mtime - a.mtime);

        if (files.length > 0) {
          try {
            const todos = JSON.parse(fs.readFileSync(path.join(todosDir, files[0].name), 'utf8'));
            const inProgress = todos.find((t) => t.status === 'in_progress');
            if (inProgress?.activeForm) {
              segments.push(bold(inProgress.activeForm));
            }
          } catch {}
        }
      }
    }

    // Directory
    segments.push(dim(path.basename(dir)));

    // Cost
    const costStr = formatCost(totalCost);
    if (costStr) segments.push(costStr);

    // Tokens
    const inStr = formatCompact(inputTokens);
    const outStr = formatCompact(outputTokens);
    if (inStr || outStr) {
      const parts = [];
      if (inStr) parts.push(`${inStr}\u2191`);
      if (outStr) parts.push(`${outStr}\u2193`);
      segments.push(dim(parts.join(' ')));
    }

    // Lines changed
    const addedParts = [];
    if (linesAdded > 0) addedParts.push(green(`+${linesAdded}`));
    if (linesRemoved > 0) addedParts.push(red(`-${linesRemoved}`));
    if (addedParts.length > 0) segments.push(addedParts.join(' '));

    // Context bar
    const ctxBar = buildContextBar(remaining);
    if (ctxBar) segments.push(ctxBar);

    // Join all segments with dimmed separator
    process.stdout.write(segments.join(` ${dim('\u2502')} `));
  } catch {
    // Silent fail - don't break statusline on parse errors
  }
});
