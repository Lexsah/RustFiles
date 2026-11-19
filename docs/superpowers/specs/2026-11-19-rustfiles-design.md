# RustFiles design

RustFiles is a cross-platform file manager in Rust. It ports the core Files experience (tabs, sidebar, navigation, views, file operations, themes) and adds an Everything-style filename index plus a WizTree-style disk analyzer. The current Files app feels slow; responsiveness is the main constraint.

The task statement is the approved scope. This document records the design used for the v0.1 implementation.

## Stack

egui 0.37 via eframe, glow renderer, Rust 1.100.

Rejected alternatives:

- WinUI keeps the original look and stays Windows-only, which is the constraint we are leaving.
- GTK feels native on Linux and awkward on Windows, and it pulls a large system toolkit into the Windows build.
- iced is a good fit for structured apps, but virtualizing tens of thousands of rows is more code and the compile cycle is heavier.
- Slint looks closer to a native toolkit, at the cost of an extra UI language and a smaller ecosystem.
- wgpu is the faster 3D renderer, and it also spends startup time compiling shaders and fails more often on remote and virtual GPUs. A file manager is a 2D list. Glow starts quickly and is enough.

The UI thread never reads the disk and never scans the index. Those jobs run on a worker thread. Directory widgets use egui's row virtualization.

## Crates

- `rustfiles-core` has no GUI and no `unsafe`. It owns the index, search, persistence, NTFS parsers, USN record parser, directory walk, folder sizes, treemap and sunburst layout, sorting, and file operations.
- `rustfiles` is the eframe app, the worker, volume detection, and the Windows volume IO.

Windows IO is the only `unsafe` code. Parsers are pure and tested on every platform. `cargo check --target x86_64-pc-windows-msvc` typechecks the Windows IO on Linux.

## Index

Each file or directory is one record: parent index, volume, flags, 64-bit file reference, size, modified time, and offsets into a name arena (original and lowercase UTF-8). Children are adjacency lists. A trigram map (`u32` packed from three lowercase UTF-8 bytes to sorted entry ids) answers substring queries of three or more characters. One- and two-character queries scan names on the worker. Results are ranked exact, then prefix, then substring, then shorter name.

The on-disk format is `RFIDX001`: magic, CRC32, volumes (mount, label, journal cursor, method), records, and the name blobs. Children, the reference map, trigrams, and folder sizes are rebuilt on load so startup does not depend on rewalking the disk.

Folder sizes are a post-order sum over the index. Hard links are stored once, under the Win32 name, so a hard-linked file is counted once.

## Filling the index

Windows NTFS: open `\\.\X:`, read the boot sector, follow `$MFT` data runs, parse `FILE` records. File ids keep the 48-bit record number and 16-bit sequence so they match the USN journal. If the open fails (not an administrator, or not NTFS), fall back to a parallel directory walk.

After a saved NTFS index loads, catch up with `FSCTL_READ_USN_JOURNAL` from the stored USN. A journal id change or an expired cursor triggers a full rescan of that volume. RustFiles does not create a USN journal.

Linux and the Windows fallback: parallel `read_dir` by directory level, do not follow symlinks, do not cross filesystem ids, and skip `proc`, `sys`, `dev`, and `run` at the root. `notify` watches volume roots recursively. If the watch fails (inotify limit), a full rescan runs every three minutes.

`RUSTFILES_INDEX_PATHS` (`|`-separated) limits which trees are indexed. The GUI uses it for the smoke test.

Directory listings come from the index when the path is present, otherwise from `read_dir` on the worker. The UI keeps the last listing per path and shows it immediately while a refresh is in flight.

## File operations

Copy, cut, and paste use an internal clipboard. Name collisions become `name (2).ext`. A cut across devices copies then deletes the source. A folder cannot be copied or moved into itself. Delete moves the path to the system trash. Rename rejects empty names and separators. New Folder creates `New folder`, then `New folder (2)`, and so on.

## Analyzer

The analyzer lists the children of a folder by rolled-up size, draws a squarified treemap in the unit square, and draws one sunburst ring. Clicking a directory drills into it. More than 200 children keep the largest 199 and group the rest as Other.

## UI

Tabs, a sidebar (favorites and volumes), back/forward/up, breadcrumbs plus an editable path, details and grid views, column sort, a preview pane (text and images), search-as-you-type, dark and light themes, and the keyboard shortcuts in the README. The analyzer replaces the file pane until the user leaves it.

## Explicitly not in v0.1

Dual pane, column view, tags, cloud providers, network locations, WSL integration, archive browsing, file hashes, an ACL editor, fanotify, creating a USN journal, NTFS attribute-list extension records, and an AppImage. The Windows installer is Inno Setup when the release image can run it; the portable zip is always produced.

## Verification

Core behavior is covered by unit tests (search, persistence, MFT parsers, USN apply, treemap area, sunburst angles, walk, copy/rename). `rustfiles --self-test` runs a temp-directory round trip. `RUSTFILES_SMOKE` drives the GUI through browse, search, and analyze, then exits 0.
