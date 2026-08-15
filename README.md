Here is the detailed list of the new and refined features in this final **v3.2.0** version of `confy`. 

I focused heavily on turning the tool into a **Dotfile Operating System**—making it smarter, safer, and completely freeze-proof.

### 🚀 Core Engine & Architecture
*   **The Monolithic Return:** Reverted from a 6-file module split back to a single, beautifully organized `main.rs`. This eliminates all `crate::` boilerplate and allows the `App` struct to use clean `&mut self` references for all state changes.

*   **Silent External Commands:** Every single external command (`zip`, `unzip`, `git`, `bat`, `chafa`, `fzf`) is now strictly wrapped in `Stdio::null()` and `Stdio::piped()`. They can no longer print ANSI escape codes or progress bars that corrupt the Ratatui alternate screen.

### 📦 Smart Deployment Flow
*   **Interactive `y/n` Deploy Confirmation:** Pressing `D` on a `.zip` file now runs a Dry Run directly in the preview pane. If successful, the bottom bar changes to: `"Deploy (y=apply, n=cancel)"`. Pressing `y` executes the deployment with timestamped backups; pressing `n` or `Esc` cancels it.
*   **Native Password Prompting:** If you press `D` on an encrypted `.zip`, `unzip` fails instantly. `confy` catches the `PASSWORD_REQUIRED` error and dynamically transforms the bottom bar into a password input field: `"🔒 enter passwd (esc=cancel)"`. Once you type the password and hit `Enter`, it re-runs the dry run seamlessly.
*   **Conflict-Aware Backups:** During `--apply`, `confy` hashes the target file on the new machine and compares it to the file inside the archive. If the hashes match (identical content), it skips the file completely—no unnecessary `.bak` files are created. If they differ, it creates a timestamped backup (e.g., `config.kdl.bak_20231024_153005`) before overwriting.

### ⏳ Version Control & History
*   **Content-Hashing Snapshots:** Snapshots are *only* taken when a file's contents actually change. It uses Rust's `DefaultHasher` to compare the current file against the latest snapshot. No disk space is wasted on empty edits.
*   **Version Diff View:** In Version Mode (`v`), press `d` to view a colored `diff` (using `diff --color=always -u`) between the selected historical version and your current file. The output is cleanly rendered in the preview pane.
*   **Symlink-Safe Restores:** When restoring a previous version, `confy` checks if the destination is a symlink. If it is, it writes *through* the symlink, ensuring your real config file is updated without breaking your folder structure.

### 📝 Persistent State & Metadata
*   **Bookmarks (`★`):** Press `b` to bookmark a file. Bookmarks are saved permanently to `~/.confy_state.json` and survive terminal restarts.
*   **File Notes (`📝`):** Press `n` to write a note for a file (e.g., "Laptop only", "Needs root"). Notes appear pinned to the top of the preview pane in yellow text.
*   **Recent Files (MRU):** The last 5 files you edited are saved to state. Press `R` (Shift+R) to open an `fzf` popup of recent files and instantly jump to them.

### ✂️ File Operations & UX
*   **Visual Select Mode (`V`):** Enter a mode where moving `j`/`k` automatically highlights files (`✓`). Pressing `Esc` clears all selections, pressing `V` again exits the mode but keeps your selections.
*   **Dynamic Cyber-Cat:** The bottom bar cat `ฅ^•ﻌ•^ฅ` changes expressions based on what you're doing. It holds scissors `✂️` when cutting, a clipboard `📋` when yanking, and sparkles `✨` when files are selected.
*   **Context-Aware Path Safety:** The `is_path_safe` function runs on all Add, Rename, and Folder Creation actions. It canonicalizes paths to block `../../` directory traversal attacks, ensuring you never accidentally escape the `~/.confy` sandbox.
*   **Smart Symlink Paste:** Cut/Yank/Paste operations safely prevent pasting a folder into itself by comparing canonical absolute paths.

### ⌨️ CLI Utilities
*   `confy -d <archive.zip> [--apply] [--pass <password>]`: Deploy archives from the command line.
*   `confy discover`: Auto-discovers and links famous dotfiles (Hyprland, Niri, Kitty, bashrc, etc.).
*   `confy doctor`: Checks for missing dependencies (`bat`, `chafa`, `fzf`, `ffmpeg`, `zip`, `unzip`, `diff`) and scans for broken symlinks.
*   `confy git-export`: Instantly initializes a git repo in `~/.confy` and commits everything for pushing to GitHub.
*   `confy -s "<password>"`: Securely saves your sudo password (enforces `0600` permissions) so you can edit root-owned files without leaving the TUI.
