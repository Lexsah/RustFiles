# RustFiles implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship RustFiles 0.1, a cross-platform file manager with an Everything-style index and a disk analyzer.

**Architecture:** `rustfiles-core` holds tested, GUI-free index, parser, walk, analyzer, and file-operation code. `rustfiles` is an eframe glow app whose worker thread owns the index. Windows NTFS and USN IO sit behind `cfg(windows)` and call the pure parsers.

**Tech Stack:** Rust 1.100, egui/eframe 0.37 (glow, x11, wayland), rayon, notify 8, trash 5, windows-sys 0.61, crc32fast, serde_json.

## Global Constraints

- All repository text (code, comments, docs, commits, PR) is English.
- One codebase builds on `windows-latest` and `ubuntu-latest`.
- The UI thread does not read the filesystem or scan the index.
- NTFS parsing and USN record parsing are pure functions with tests that run on Linux.
- MFT or USN failure falls back to a directory walk. Do not create a USN journal.
- Delete goes to the trash unless the user confirms a permanent delete.
- v0.1 prefers a solid core over Files features listed as out of scope in the design spec.

---

### Task 1: Workspace and core model

**Files:** `Cargo.toml`, `rust-toolchain.toml`, `crates/rustfiles-core/**`, `LICENSE`, `.gitignore`

**Interfaces:**

- `IndexBuilder::add_volume` / `add_file` / `finish` -> `Index`
- `Index::search(&str, usize) -> Vec<SearchHit>`
- `Index::find_path(&str) -> Option<u32>`
- `Index::list_dir(u32, bool) -> Vec<FileRow>`
- `Index::save` / `Index::load`
- `Index::ensure_sizes` / `folder_size`
- `Index::apply_usn` / `reconcile_children`

- [ ] Write failing tests for natural sort, trigram search, paths, sizes, persistence, and reconcile.
- [ ] Implement the index until `cargo test -p rustfiles-core` passes.

### Task 2: NTFS, USN, walk, operations, layouts

**Files:** `crates/rustfiles-core/src/{mft,usn,walk,ops,analyze,sort}.rs`

**Interfaces:**

- `parse_boot_sector`, `parse_data_runs`, `apply_fixup`, `parse_mft_stream`, `read_runs_to_vec`
- `parse_usn_page(&[u8]) -> (i64, Vec<UsnRecord>)`
- `walk_tree(&Path, &AtomicU64) -> Vec<RawFile>`
- `copy_into`, `paste`, `rename_path`, `create_folder`, `delete_to_trash`
- `squarify`, `sunburst`

- [ ] Tests first for boot sector, data runs, fixup, a two-record MFT image, USN create/rename/delete, walk parents, copy/rename, treemap area, sunburst sweep.
- [ ] Implement until the same test command passes.

### Task 3: Application

**Files:** `crates/rustfiles/**`

**Interfaces:** worker `Command` / `Event` channel; `RUSTFILES_INDEX_PATHS`; `RUSTFILES_SMOKE`; `--self-test`; `--version`.

- [ ] Sidebar, tabs, breadcrumbs, details and grid, search, preview, operations, shortcuts, themes, analyzer.
- [ ] Windows: MFT read, USN catch-up, drive list. Other platforms: `/proc/mounts` and the walk fallback.
- [ ] `cargo test --workspace` and `cargo build --release` pass.
- [ ] `--self-test` passes. `RUSTFILES_SMOKE` exercises browse, search, and analyze.

### Task 4: CI, packages, docs

**Files:** `.github/workflows/ci.yml`, `packaging/*`, `README.md`

- [ ] CI builds and tests on Windows and Ubuntu, runs `--self-test`, Linux GUI smoke, and uploads release zip/tar plus an Inno Setup installer on version tags.
- [ ] README documents features, gaps, build steps, shortcuts, and the index.

### Execution note

The crates share one workspace and the GUI depends on the core types, so the tasks are sequential. They are implemented in this session (executing-plans) rather than by parallel implementers. A final review uses the code-review skill.
