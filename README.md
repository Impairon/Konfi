                                                       ﷽

## confy: The Config Manager 
`confy` is a blazingly fast, single-file Rust TUI for managing symlinks in `~/.configz`. It's inspired by tools like `yazi` and `nix os home manager`, but strictly for your configuration files. No bloat, no unnecessary abstractions. It just sits there, looks sharp, and manages your files.

## 🐾 The Clowder (Features)

- 🌳 **Tree View:** Expand directories inline. Press `Enter` on a folder to branch out its contents and survey your territory.
- 👁️ **Sneak Peeks:** Previews text (with `bat`), images (with `chafa`), and video thumbnails (with `ffmpeg`). It sees the red dot before you do.
- 🔍 **Hunt Mode (Search):** Recursive search. Type `/` and pounce on the exact file you want. It highlights the matched text so you don't need night vision.
- 🐈‍⬛ **FZF Integration:** Press `z` while adding a file to summon `fzf`. It searches your entire home directory for the perfect snack.
- 🎨 **Auto-Theming:** Sniffs out your terminal's Truecolor/ANSI capabilities. If you use Catppuccin Mocha, it matches it perfectly. If you use a light theme, it adapts. No hardcoded RGB traps here.
- 🧹 **Clean Litter Box:** Delete files safely. It requires pressing `d` twice to confirm, because cats are naturally cautious creatures.
- ⌨️ **CLI Speedruns:** Link or open files directly from the terminal without even opening the TUI.

## 🥛 Prerequisites
Make sure you just have `fzf`
## 📦 Installation

Build the optimized binary and put it in your local bin:

```bash
git clone https://github.com/Impairon/confy.git
cd confy/src
cargo build --release
cp target/release/confy ~/.local/bin/confy
```

*(Make sure `~/.local/bin` is in your `$PATH`)*

## ⌨️ Usage

### CLI Mode (For when you're too lazy to open the UI)

**Create a symlink instantly:**
```bash
confy -l ~/.config/niri/config.kdl -n niri_config
```

**Open a file directly in your `$EDITOR` (bypasses the TUI completely):**
```bash
confy -o niri_config
```

**Open the TUI and automatically expand a specific folder:**
```bash
confy -o my_folder
```

### TUI Mode (For when you want to play with your food)

| Key | Action | 
| --- | ------ | 
| `j` / `k` / `↑` / `↓` | Navigate | 
| `Enter` | Expand folder / Open file |
| `a` | Add a new symlink | 
| `r` | Rename a symlink |
| `d` | Delete (press twice) |
| `/` | Search |
| `q` | Quit |

---
<img width="100%" height="75" alt="cat_line" src="https://github.com/user-attachments/assets/9fdb860f-a8e7-47da-941b-e91ed0646feb" /><svg width="600" height="75" viewBox="0 0 600 75" version="1.1" xmlns="http://www.w3.org/2000/svg" style="stroke-linecap: round; stroke-linejoin: round; stroke-miterlimit: 1.5;">
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


