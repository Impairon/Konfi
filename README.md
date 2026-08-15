
# Confy just got a huge upgrade and new features:

### 🌳 File Tree & Navigation
*   **Inline Tree View:** Folders expand and collapse directly in the list without needing a separate pane.
*   **Smart Expansion Memory:** If the filesystem changes externally, the tool refreshes the list but remembers exactly which folders you had open.
*   **Mouse Support:** Scroll wheel works natively for navigating the file list.
*   **Hidden Files Toggle:** Press `.` to show/hide hidden files (dotfiles).

### 📁 File & Folder Operations
*   **Add Symlink (`a`):** Two-step prompt for Path and Alias. Supports path-aware naming (e.g., typing `work/niri.kdl` creates the `work/` folder if it doesn't exist).
*   **Tab Completion (`Tab`):** Context-aware `fzf` integration. Press `Tab` while typing a path to search the directory you are currently typing in.
*   **Create Folder (`f`):** Instantly create a new folder at the root of `~/.configz`.
*   **Rename (`r`):** Rename files or folders. Also supports path-aware naming to move files via rename.
*   **Cut & Paste (`c` / `p`):** Cut a file/folder, navigate, and press `p` to paste. Safely prevents pasting a folder into itself.
*   **Yank & Paste (`y` / `p`):** Copy a file/folder, navigate, and press `p` to duplicate it. Uses `cp -r` for directories.
*   **Safe Delete (`d`):** Press `d` once to arm, press `d` again to confirm. Safely handles symlinks, directories, and files.

### ⏳ Smart Version Control
*   **Content-Hashing Snapshots:** Versions are saved *only* when you close your editor, and *only* if the file's contents actually changed (using Rust's `DefaultHasher`).
*   **Jujutsu-style Timeline (`v`):** View a clean, chronological timeline of file versions directly in the preview pane.
*   **Surgical Restores (`Enter`):** Restore a specific file to a previous state. Symlinks are protected during restoration so your config structure isn't broken.
*   **Configurable Limits (`-v`):** Set how many versions to keep (default 4). Old snapshots are automatically cleaned up.

### 🔒 Sudo & Shell Integration
*   **Sudo suuport:** If a file needs root permissions, a password prompt appears in the bottom bar. Empty password = view-only mode. Saved passwords (`-s`) are tried silently first.
*   **Shell Mode (`!`):** Run shell commands directly against the highlighted file. The output is captured and displayed in a "pinned" preview pane with a header and footer, so you can read long outputs without leaving the TUI. Press `Esc` to return to normal preview.
*   **Sudo Password Manager (`-s`):** Save or delete your sudo password via CLI (enforced `0600` permissions for security).

### 🛡️ Security & Robustness
*   **Path Traversal Protection (`is_path_safe`):** All add, rename, and folder creation actions are sanitized. You cannot use `../../` to escape the `~/.configz` directory.
*   **Symlink Target Viewer:** If a file is a symlink, the preview pane displays exactly where it points (e.g., `Symlink -> /home/user/.config/niri/config.kdl`).
*   **Binary File Protection:** Uses `bat` for syntax highlighting, but intercepts binary file warnings to prevent terminal corruption.

### 🎨 UI, Theming & Quality of Life
*   **Terminal Theme Adaptation:** Detects `COLORTERM` to use Truecolor (Catppuccin Mocha) or falls back gracefully to ANSI 256/Standard colors. Matches your terminal perfectly.
*   **Help Modal (`?`):** A clean, centered popup lists all keybindings so the bottom bar stays uncluttered.
*   **Dynamic Cats:** The bottom bar features a kitty `ฅ^•ﻌ•^ฅ` that changes expressions based on the mode (e.g., holding scissors `✂️` when cutting, a clipboard `📋` when yanking, a clock `⏳` when viewing versions).
*   **Escape any operation:** Now a `ESC` option is available for operations you do so you can aport it 
*   **Fixed Column Widths:** The file list is locked to 45 characters to ensure the UI never squishes or breaks on smaller terminals.

### ⌨️ Command Line Interface (CLI)
*   `confy` : Launch the TUI.
*   `confy -l <path> -n <name>` : Create a symlink instantly from the terminal (creates parent folders).
*   `confy -o <file/folder>` : Open a file in `$EDITOR` or launch the TUI with a folder expanded.
*   `confy -v <number>` : Set version limit.
*   `confy -s "<password>"` : Save sudo password.
*   `confy -s ""` : Delete saved sudo password.
*   `confy -h` : Show help.
