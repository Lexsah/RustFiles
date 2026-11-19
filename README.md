# RustFiles

RustFiles is a cross-platform file manager written in Rust. It ports the core of [Files](https://github.com/files-community/Files) (tabs, sidebar, navigation, details and grid views, preview, common file operations, themes) and adds two things the original does not: an Everything-style filename index, and a disk analyzer with a treemap and a sunburst.

Files is a C# / WinUI app and only runs on Windows. RustFiles is one codebase for Windows and Linux. The goal of this port is speed: the window thread never reads the disk and never scans the index.

## Why egui

The UI is [egui](https://github.com/emilk/egui) 0.37 on [eframe](https://github.com/emilk/egui/tree/master/crates/eframe), using the glow renderer.

- One immediate-mode UI on both operating systems. There is no WinUI or GTK split, and no second widget toolkit to keep in sync.
- Directory listings are virtualized. A folder with tens of thousands of entries only builds the rows on screen.
- Glow is a thin OpenGL layer. It starts quickly and does not compile a wgpu shader pipeline on launch, which matters on older GPUs and remote desktops. A file manager is a 2D list; the slow part is the disk, and that work is off the UI thread.

iced and Slint were the other serious options. iced is a good fit for structured apps, but virtualizing huge lists is more code. Slint looks closer to a native toolkit and adds a separate UI language. WinUI cannot leave Windows, which is the constraint this port exists to remove.

## Features in 0.1

- Tabs, back / forward / up, breadcrumbs, and an editable path
- Sidebar with favorites (pin the current folder) and detected drives or mount points
- Details and grid views, with name / date / type / size sorting and directories kept first
- Preview pane for text files and png, jpeg, gif, and bmp images
- Copy, cut, paste, rename, New Folder, and delete to the system trash (Shift+Delete asks before a permanent delete)
- Dark and light themes, saved between launches
- Keyboard shortcuts (F1 lists them in the app)
- Incremental filename search over a persistent index
- Disk analyzer: size-sorted children, squarified treemap, single-ring sunburst, click to drill in

## Not ported yet

Dual pane, column view, tags, cloud providers, network locations, WSL integration, archive browsing, file hashes, and an ACL editor. NTFS attribute-list extension records (names that live only outside the base file record) are skipped. Hard links are stored once, under the Win32 name. Linux does not use fanotify, because that needs root. There is no AppImage; Linux ships as a tarball with a `.desktop` file. The Windows installer is Inno Setup, and a portable zip is always produced.

## Index

The index is a flat table of records plus a trigram map.

Each record stores a parent id, a volume id, flags, a 64-bit file reference, size, modified time, and offsets into a name arena (original case and lowercase UTF-8). Children are adjacency lists. Folder sizes are a post-order sum, so the analyzer does not walk the disk again.

Search extracts the lowercase UTF-8 trigrams of the query (three or more bytes), intersects the sorted posting lists, and checks the remaining candidates with a real substring test. One- and two-character queries scan names on the worker thread. Hits are ranked exact match, then prefix, then substring, then shorter name. The UI receives at most 400 hits.

The on-disk file is `RFIDX001`: magic, CRC32, volume records (mount, label, USN cursor, how the volume was read), and the file records. Children, the trigram map, and folder sizes are rebuilt when the file is loaded, which is much faster than walking the disk again. The file lives in `%LOCALAPPDATA%\RustFiles\index.rfi` on Windows and `$XDG_CACHE_HOME/rustfiles/index.rfi` (or `~/.cache/rustfiles/index.rfi`) on Linux.

### Windows

For an NTFS volume RustFiles opens `\\.\X:`, reads the boot sector, follows the `$MFT` data runs, and parses `FILE` records in chunks. The file reference keeps the 48-bit record number and the 16-bit sequence number so it matches the USN change journal. After a saved index loads, the app reads `FSCTL_READ_USN_JOURNAL` from the stored cursor and applies creates, renames, and deletes. If the journal id changed, or the cursor is too old, that volume is walked again.

Opening the raw volume and the journal requires an elevated process. Without it, or on FAT, exFAT, or a network drive, RustFiles walks the directory tree in parallel and says so in the status bar. RustFiles does not create a USN journal.

### Linux

Volumes come from `/proc/mounts` (ext4, btrfs, xfs, NTFS, and the other common disk filesystems). The walk is parallel by directory level, does not follow symlinks, does not cross onto another device, and skips `/proc`, `/sys`, `/dev`, `/run`, and `/snap`. `notify` then watches each volume recursively (inotify on Linux). If a watch cannot be installed, because the inotify limit is too low, the volume is rescanned every three minutes.

Set `RUSTFILES_INDEX_PATHS` to a `|`-separated list of directories to index only those trees. The GUI smoke test uses this.

Directory listings come from the index when the path is present, and from `read_dir` on the worker otherwise. The window keeps the last listing for each path and shows it immediately while a refresh is in flight.

## Build

Install a stable Rust toolchain (the repository pins 1.100.0) and a C build environment.

Linux also needs the glow / winit libraries:

```bash
sudo apt-get install pkg-config libx11-dev libxkbcommon-dev libwayland-dev \
  libgl1-mesa-dev libegl1-mesa-dev libxcb1-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libxcursor-dev libxi-dev
cargo run -p rustfiles --release
```

Windows, from a Visual Studio developer prompt or a GitHub `windows-latest` runner:

```bat
cargo run -p rustfiles --release
```

Run the tests and a headless round trip of walk, search, analyze, copy, and index reload:

```bash
cargo test --workspace
cargo run -p rustfiles -- --self-test
```

`cargo run -p rustfiles -- --version` prints the version without opening a window.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+T / Ctrl+W / Ctrl+Tab | New tab, close tab, next tab |
| Alt+Left / Alt+Right / Alt+Up | Back, forward, parent |
| Backspace | Parent |
| Ctrl+L | Edit the path |
| Ctrl+F | Focus search |
| Enter | Open the selection |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy, cut, paste |
| Ctrl+Shift+N | New folder |
| F2 | Rename |
| Delete | Move to trash |
| Shift+Delete | Permanent delete, after a confirmation |
| Ctrl+A | Select all |
| F5 | Refresh the folder |
| F1 | Shortcut list |

## License

MIT. See [LICENSE](LICENSE). Files is also MIT; this repository is a new implementation, not a translation of the C# source.
