
                                           ﷽

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

# set up
```bash
git clone https://github.com/Impairon/confy.git
cd confy
cargo build --release
cp target/release/confy ~/.local/bin/
```
<img width="100%" height="75" alt="cat_line" src="https://github.com/user-attachments/assets/6a269eeb-260e-47ba-b028-608b8b7d4546" />

<svg width="600" height="75" viewBox="0 0 600 75" version="1.1" xmlns="http://www.w3.org/2000/svg" style="stroke-linecap: round; stroke-linejoin: round; stroke-miterlimit: 1.5;">
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M105.809,48.397C105.809,44.506 102.473,43.931 102.473,33.503" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M109.397,38.324L109.397,48.321" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M112.883,48.152C112.883,44.717 115.053,40.554 115.053,35.084C115.053,29.613 114.393,24.795 114.216,21.81" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M112.951,22.241C112.951,22.241 116.335,21.976 117.504,16.695" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M107.788,11.843C107.788,11.843 106.369,7.434 105.169,7.434C103.969,7.434 101.87,13.187 101.87,21.862C101.87,24.103 90.181,29.985 92.659,43.571C93.057,45.751 94.053,49.908 94.053,49.924C94.053,49.94 96.571,59.453 91.184,59.453C90.063,59.453 89.526,58.833 88.405,58.833C87.285,58.833 86.381,59.598 86.381,60.591C86.381,61.584 87.491,64.025 91.446,64.025C98.593,64.025 98.865,58.038 98.865,54.158C98.865,50.278 98.829,51.479 98.829,50.844C98.829,48.717 100.601,48.284 101.259,48.043" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <ellipse transform="matrix(1.00474,-0.404483,0.370766,0.920982,85.4108,49.8267)" cx="111.892" cy="15.766" rx="1.032" ry="1.449" style="fill: rgb(47, 44, 62);"/>
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M110.074,10.347C113.617,10.347 114.448,14.635 117.14,14.635" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <path transform="matrix(1,0,0,1,92.3579,4.11772)" d="M112.568,9.074C112.568,9.074 111.553,6.74 110.677,6.74C109.801,6.74 108.537,9.169 108.537,9.169" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 1.5px;"/>
    <path transform="matrix(3.96613,0,0,5.89452,-177.012,-336.835)" d="M93.717,66.428L195.647,66.428" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 0.3px;"/>
    <path transform="matrix(1.78906,0,0,2.78204,-166.7,-130.078)" d="M93.717,66.428L195.647,66.428" style="fill: none; stroke: rgb(110, 108, 126); stroke-width: 0.64px;"/>
</svg>
