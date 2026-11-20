//! In-memory filename index.
//!
//! Records stay in a flat table. The trigram map, child lists, and folder
//! sizes are derived and can be rebuilt after a load.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::PathBuf;

use rustc_hash::FxHashMap;

use crate::flags::{self, DIRECTORY, TOMBSTONE};
use crate::model::{FileRow, RawFile, SearchHit, Volume};
use crate::usn::{
    UsnRecord, USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE, USN_REASON_RENAME_NEW_NAME,
    USN_REASON_RENAME_OLD_NAME,
};

pub const NO_PARENT: u32 = u32::MAX;
pub const METHOD_WALK: u8 = 0;
pub const METHOD_MFT: u8 = 1;

/// 32 bytes. Names live in one arena. Children are a sibling list, not a
/// `HashMap` of `Vec`s. File references are `id + 1` unless `file_refs` is filled.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) parent: u32,
    pub(crate) name_start: u32,
    pub(crate) first_child: u32,
    pub(crate) next_sibling: u32,
    pub(crate) volume: u16,
    pub(crate) flags: u16,
    pub(crate) name_len: u16,
    pub(crate) _pad: u16,
    pub(crate) size_lo: u32,
    pub(crate) modified: u32,
}

/// File-reference lookup. Record numbers that sit near the entry id live in a
/// flat table (4 bytes each). Outliers stay in a hash map so a sparse NTFS
/// reference cannot allocate a giant array.
#[derive(Clone, Debug)]
struct Refs {
    dense: Vec<Vec<u32>>,
    sparse: FxHashMap<(u16, u64), u32>,
}

const REF_ABSENT: u32 = u32::MAX;

impl Refs {
    fn new() -> Self {
        Self {
            dense: Vec::new(),
            sparse: FxHashMap::default(),
        }
    }

    fn clear(&mut self) {
        self.dense.clear();
        self.sparse.clear();
    }

    fn get(&self, volume: u16, file_ref: u64) -> Option<u32> {
        let slot = (file_ref & 0x0000_FFFF_FFFF_FFFF) as usize;
        if let Some(table) = self.dense.get(volume as usize) {
            if let Some(&id) = table.get(slot) {
                if id != REF_ABSENT {
                    return Some(id);
                }
            }
        }
        self.sparse.get(&(volume, file_ref)).copied()
    }

    fn insert(&mut self, volume: u16, file_ref: u64, id: u32) {
        let slot_key = file_ref & 0x0000_FFFF_FFFF_FFFF;
        let near = slot_key <= (id as u64).saturating_mul(3).saturating_add(8_192)
            && slot_key <= 8_000_000;
        if near {
            let slot = slot_key as usize;
            if self.dense.len() <= volume as usize {
                self.dense.resize(volume as usize + 1, Vec::new());
            }
            let table = &mut self.dense[volume as usize];
            if table.len() <= slot {
                table.resize(slot + 1, REF_ABSENT);
            }
            table[slot] = id;
            self.sparse.remove(&(volume, file_ref));
        } else {
            self.sparse.insert((volume, file_ref), id);
        }
    }

    fn bytes(&self) -> usize {
        let mut total = self.sparse.capacity()
            * (std::mem::size_of::<(u16, u64)>() + std::mem::size_of::<u32>());
        for table in &self.dense {
            total += table.capacity() * std::mem::size_of::<u32>();
        }
        total
    }
}

#[derive(Clone, Debug)]
pub struct Index {
    pub(crate) volumes: Vec<Volume>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) names: Vec<u8>,
    /// Empty when every `file_ref` is `id + 1` (the directory walk). Otherwise one `u64` per entry.
    pub(crate) file_refs: Vec<u64>,
    /// Files whose size does not fit in `u32`.
    pub(crate) big_sizes: FxHashMap<u32, u64>,
    refs: Refs,
    trigrams: FxHashMap<u32, Vec<u32>>,
    trigrams_ready: bool,
    sizes: Vec<u64>,
    sizes_valid: bool,
    pub(crate) next_file_ref: u64,
}

#[derive(Clone, Debug)]
pub struct LiveChild {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub flags: u16,
}

pub struct IndexBuilder {
    index: Index,
    /// `(entry id, parent file reference)` for records whose parent was not
    /// inserted yet. A directory walk resolves parents immediately and leaves
    /// this empty, so the build does not keep a second hash map of every file.
    deferred: Vec<(u32, u64)>,
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self {
            index: Index::empty(),
            deferred: Vec::new(),
        }
    }

    pub fn add_volume(&mut self, mount: String, label: String, case_insensitive: bool) -> u16 {
        let id = self.index.volumes.len() as u16;
        self.index.volumes.push(Volume {
            mount,
            label,
            root: NO_PARENT,
            journal_id: 0,
            next_usn: 0,
            case_insensitive,
            method: METHOD_WALK,
        });
        id
    }

    pub fn set_journal(&mut self, volume: u16, journal_id: u64, next_usn: i64, method: u8) {
        if let Some(volume) = self.index.volumes.get_mut(volume as usize) {
            volume.journal_id = journal_id;
            volume.next_usn = next_usn;
            volume.method = method;
        }
    }

    pub fn add_file(&mut self, volume: u16, file: RawFile) {
        if self.index.lookup_ref(volume, file.file_ref).is_some() {
            return;
        }
        let id = self.index.entries.len() as u32;
        let parent = if file.parent_ref == file.file_ref {
            NO_PARENT
        } else {
            self.index
                .lookup_ref(volume, file.parent_ref)
                .filter(|parent| *parent != id)
                .unwrap_or(NO_PARENT)
        };
        if parent == NO_PARENT && file.parent_ref != file.file_ref {
            self.deferred.push((id, file.parent_ref));
        }
        self.index.push_entry(volume, &file, parent);
        self.index.remember_ref(volume, file.file_ref, id);
        if file.parent_ref == file.file_ref {
            let stored = &mut self.index.volumes[volume as usize];
            if stored.root == NO_PARENT {
                stored.root = id;
            }
        }
    }

    pub fn finish(mut self) -> Index {
        for (id, parent_ref) in self.deferred {
            let volume = self.index.entries[id as usize].volume;
            let resolved = match self.index.lookup_ref(volume, parent_ref) {
                Some(parent) if parent != id => parent,
                _ => {
                    let root = self.index.volumes[volume as usize].root;
                    if root == id {
                        NO_PARENT
                    } else {
                        root
                    }
                }
            };
            self.index.entries[id as usize].parent = resolved;
        }
        for volume_index in 0..self.index.volumes.len() {
            if self.index.volumes[volume_index].root == NO_PARENT {
                if let Some(position) = self
                    .index
                    .entries
                    .iter()
                    .position(|entry| entry.volume as usize == volume_index)
                {
                    self.index.volumes[volume_index].root = position as u32;
                }
            }
        }
        self.index.rebuild_derived();
        self.index.entries.shrink_to_fit();
        self.index.names.shrink_to_fit();
        self.index.next_file_ref = (0..self.index.entries.len() as u32)
            .map(|id| self.index.file_ref_at(id))
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.index
    }
}

impl Default for IndexBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Index {
    pub(crate) fn empty() -> Self {
        Self {
            volumes: Vec::new(),
            entries: Vec::new(),
            names: Vec::new(),
            file_refs: Vec::new(),
            big_sizes: FxHashMap::default(),
            refs: Refs::new(),
            trigrams: FxHashMap::default(),
            trigrams_ready: false,
            sizes: Vec::new(),
            sizes_valid: false,
            next_file_ref: 1,
        }
    }

    pub fn volumes(&self) -> &[Volume] {
        &self.volumes
    }

    /// Approximate heap use of the live index, excluding allocator overhead.
    pub fn resident_bytes(&self) -> usize {
        let mut total = self.entries.len() * std::mem::size_of::<Entry>();
        total += self.names.capacity();
        total += self.file_refs.capacity() * std::mem::size_of::<u64>();
        total += self.big_sizes.capacity() * 16;
        total += self.refs.bytes();
        if self.trigrams_ready {
            total += self.trigrams.capacity() * 32;
            for list in self.trigrams.values() {
                total += list.capacity() * std::mem::size_of::<u32>();
            }
        }
        total += self.sizes.capacity() * std::mem::size_of::<u64>();
        total
    }

    pub fn entry_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.flags & TOMBSTONE == 0)
            .count()
    }

    pub fn name_of(&self, id: u32) -> &str {
        self.text(id)
    }

    pub fn set_journal(&mut self, volume: u16, journal_id: u64, next_usn: i64) {
        if let Some(volume) = self.volumes.get_mut(volume as usize) {
            volume.journal_id = journal_id;
            volume.next_usn = next_usn;
        }
    }

    pub fn search(&mut self, query: &str, limit: usize) -> Vec<SearchHit> {
        self.ensure_trigrams();
        if limit == 0 {
            return Vec::new();
        }
        let raw = query.trim().to_lowercase();
        if raw.is_empty() {
            return Vec::new();
        }
        let (dir_filter, needle) = split_query(&raw);
        if needle.is_empty()
            && dir_filter
                .as_ref()
                .map(|dir| dir.is_empty())
                .unwrap_or(true)
        {
            return Vec::new();
        }
        let mut heap = BinaryHeap::new();
        if needle.len() >= 3 {
            let grams = unique_trigrams(needle.as_bytes());
            let mut lists = Vec::new();
            let mut missing = grams.is_empty();
            for gram in &grams {
                match self.trigrams.get(gram) {
                    Some(list) if !list.is_empty() => lists.push(list.as_slice()),
                    _ => {
                        missing = true;
                        break;
                    }
                }
            }
            if !missing {
                for id in intersect(&lists) {
                    self.consider(&mut heap, id, &needle, dir_filter.as_deref(), limit);
                }
            }
        } else {
            for id in 0..self.entries.len() as u32 {
                self.consider(&mut heap, id, &needle, dir_filter.as_deref(), limit);
            }
        }
        let mut ranked = heap.into_vec();
        ranked.sort();
        ranked.into_iter().map(|rank| self.hit(rank.id)).collect()
    }

    pub fn find_path(&self, path: &str) -> Option<u32> {
        let (volume_index, insensitive) = self.best_volume(path)?;
        let mount = &self.volumes[volume_index].mount;
        let rest = remainder(path, mount, insensitive);
        let mut current = self.volumes[volume_index].root;
        if current == NO_PARENT || !self.alive(current) {
            return None;
        }
        if rest.is_empty() {
            return Some(current);
        }
        let sep = separator(mount);
        for component in rest.split(sep) {
            if component.is_empty() {
                continue;
            }
            current = self.child_named(current, component, insensitive)?;
        }
        Some(current)
    }

    pub fn path(&self, id: u32) -> String {
        if !self.contains(id) {
            return String::new();
        }
        let volume = &self.volumes[self.entries[id as usize].volume as usize];
        let sep = separator(&volume.mount);
        let mut names = Vec::new();
        let mut current = id;
        for _ in 0..4096 {
            let name = self.name_of(current);
            if !name.is_empty() && name != "." && name != ".." {
                names.push(name.to_string());
            }
            let parent = self.entries[current as usize].parent;
            if parent == NO_PARENT || parent == current || !self.contains(parent) {
                break;
            }
            current = parent;
        }
        names.reverse();
        let mut out = volume.mount.clone();
        if out.is_empty() {
            out.push(sep);
        }
        if !names.is_empty() && !out.ends_with(sep) {
            out.push(sep);
        }
        for (index, name) in names.iter().enumerate() {
            if index > 0 {
                out.push(sep);
            }
            out.push_str(name);
        }
        out
    }

    /// Alive entries in the subtree, including `id` itself.
    pub fn subtree_items(&self, id: u32) -> u64 {
        let mut stack = vec![id];
        let mut count = 0u64;
        while let Some(current) = stack.pop() {
            if !self.alive(current) {
                continue;
            }
            count += 1;
            stack.extend(self.child_ids(current));
        }
        count
    }

    pub fn list_dir(&mut self, id: u32, show_hidden: bool) -> Vec<FileRow> {
        self.ensure_sizes();
        let children = self.child_ids(id);
        let mut rows = Vec::new();
        for child in children {
            if !self.alive(child) {
                continue;
            }
            let name = self.name_of(child).to_string();
            let flags = self.entries[child as usize].flags;
            let hidden = flags::is_hidden_name(&name, flags);
            if hidden && !show_hidden {
                continue;
            }
            let is_dir = flags & DIRECTORY != 0;
            let size = if is_dir {
                self.sizes.get(child as usize).copied().unwrap_or(0)
            } else {
                self.size_at(child as usize)
            };
            rows.push(FileRow {
                name,
                path: PathBuf::from(self.path(child)),
                is_dir,
                hidden,
                size,
                modified: u64::from(self.entries[child as usize].modified),
            });
        }
        rows
    }

    pub fn ensure_sizes(&mut self) {
        if self.sizes_valid && self.sizes.len() == self.entries.len() {
            return;
        }
        let count = self.entries.len();
        self.sizes = vec![0; count];
        let mut state = vec![0u8; count];
        for start in 0..count {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![start];
            while let Some(&id) = stack.last() {
                if state[id] == 2 {
                    stack.pop();
                    continue;
                }
                let tombstone = self.entries[id].flags & TOMBSTONE != 0;
                let is_dir = self.entries[id].flags & DIRECTORY != 0;
                if tombstone {
                    state[id] = 2;
                    self.sizes[id] = 0;
                    stack.pop();
                    continue;
                }
                if state[id] == 0 {
                    state[id] = 1;
                    if is_dir {
                        let pending: Vec<usize> = self
                            .child_ids(id as u32)
                            .into_iter()
                            .map(|child| child as usize)
                            .filter(|child| *child < count && state[*child] == 0)
                            .collect();
                        if !pending.is_empty() {
                            stack.extend(pending);
                            continue;
                        }
                    }
                }
                self.sizes[id] = if is_dir {
                    self.child_ids(id as u32).iter().fold(0u64, |sum, child| {
                        sum.saturating_add(self.sizes.get(*child as usize).copied().unwrap_or(0))
                    })
                } else {
                    self.size_at(id)
                };
                state[id] = 2;
                stack.pop();
            }
        }
        self.sizes_valid = true;
    }

    pub fn folder_size(&self, id: u32) -> u64 {
        self.sizes.get(id as usize).copied().unwrap_or(0)
    }

    pub fn child_weights(&mut self, id: u32) -> Vec<crate::analyze::Weighted> {
        self.ensure_sizes();
        let children = self.child_ids(id);
        let mut items: Vec<_> = children
            .into_iter()
            .filter(|child| self.alive(*child))
            .map(|child| crate::analyze::Weighted {
                key: child,
                name: self.name_of(child).to_string(),
                size: self.folder_size(child).max(if self.is_dir(child) {
                    0
                } else {
                    self.size_at(child as usize)
                }),
            })
            .filter(|item| item.size > 0)
            .collect();
        items.sort_by(|a, b| b.size.cmp(&a.size).then(a.name.cmp(&b.name)));
        const MAX_CELLS: usize = 200;
        if items.len() > MAX_CELLS {
            let rest: u64 = items[MAX_CELLS - 1..].iter().map(|item| item.size).sum();
            items.truncate(MAX_CELLS - 1);
            items.push(crate::analyze::Weighted {
                key: NO_PARENT,
                name: "Other".to_string(),
                size: rest,
            });
        }
        items
    }

    pub fn apply_usn(&mut self, volume: u16, record: &UsnRecord) {
        let delete = record.reason & USN_REASON_FILE_DELETE != 0;
        let old_name = record.reason & USN_REASON_RENAME_OLD_NAME != 0;
        let create = record.reason & (USN_REASON_FILE_CREATE | USN_REASON_RENAME_NEW_NAME) != 0;
        if delete && !create {
            if let Some(id) = self.lookup_ref(volume, record.file_ref) {
                self.tombstone_tree(id);
            }
            return;
        }
        if old_name && !create {
            if let Some(id) = self.lookup_ref(volume, record.file_ref) {
                self.tombstone(id);
            }
        }
        if create {
            let flags = flags::map_win_attrs(record.attributes);
            self.upsert_ref(
                volume,
                record.file_ref,
                record.parent_ref,
                &record.name,
                0,
                record.timestamp,
                flags,
            );
        }
    }

    pub fn set_metadata(&mut self, volume: u16, file_ref: u64, size: u64, modified: u64) -> bool {
        let Some(id) = self.lookup_ref(volume, file_ref) else {
            return false;
        };
        if !self.alive(id) {
            return false;
        }
        if self.entries[id as usize].flags & DIRECTORY == 0 {
            self.set_size_at(id as usize, size);
        }
        self.entries[id as usize].modified = modified.min(u64::from(u32::MAX)) as u32;
        self.sizes_valid = false;
        true
    }

    pub fn path_for_ref(&self, volume: u16, file_ref: u64) -> Option<String> {
        let id = self.lookup_ref(volume, file_ref)?;
        if !self.alive(id) {
            return None;
        }
        Some(self.path(id))
    }

    /// Replace the direct children of `dir` with `live`. Newly created directories are returned
    /// so the caller can walk them.
    pub fn reconcile_children(&mut self, dir: u32, live: &[LiveChild]) -> Vec<u32> {
        if !self.contains(dir) || !self.alive(dir) {
            return Vec::new();
        }
        let volume = self.entries[dir as usize].volume;
        let insensitive = self.volumes[volume as usize].case_insensitive;
        let existing = self.child_ids(dir);
        let mut used = vec![false; live.len()];
        for child in existing {
            if !self.alive(child) {
                continue;
            }
            let current_name = self.name_of(child).to_string();
            let found = live.iter().enumerate().find(|(index, item)| {
                !used[*index] && names_equal(&current_name, &item.name, insensitive)
            });
            if let Some((index, item)) = found {
                used[index] = true;
                self.update_child(child, item);
            } else {
                self.tombstone_tree(child);
            }
        }
        let mut created_dirs = Vec::new();
        for (index, item) in live.iter().enumerate() {
            if used[index] {
                continue;
            }
            let mut item_flags = item.flags;
            if item.is_dir {
                item_flags |= DIRECTORY;
            }
            let id = self.alloc(
                volume,
                dir,
                self.next_file_ref,
                &item.name,
                if item.is_dir { 0 } else { item.size },
                item.modified,
                item_flags,
            );
            if item.is_dir {
                created_dirs.push(id);
            }
        }
        self.sizes_valid = false;
        created_dirs
    }

    fn update_child(&mut self, id: u32, item: &LiveChild) {
        let mut flags = item.flags;
        if item.is_dir {
            flags |= DIRECTORY;
        }
        flags &= !TOMBSTONE;
        if self.name_of(id) != item.name {
            self.clear_trigrams(id);
            self.set_name(id, &item.name);
            self.add_trigrams(id);
        }
        self.entries[id as usize].flags = flags;
        self.entries[id as usize].modified = item.modified.min(u64::from(u32::MAX)) as u32;
        if !item.is_dir {
            self.set_size_at(id as usize, item.size);
        }
        self.sizes_valid = false;
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_ref(
        &mut self,
        volume: u16,
        file_ref: u64,
        parent_ref: u64,
        name: &str,
        size: u64,
        modified: u64,
        flags: u16,
    ) {
        let parent = if parent_ref == file_ref {
            NO_PARENT
        } else {
            self.lookup_ref(volume, parent_ref).unwrap_or(NO_PARENT)
        };
        if let Some(id) = self.lookup_ref(volume, file_ref) {
            let was_dead = self.entries[id as usize].flags & TOMBSTONE != 0;
            self.clear_trigrams(id);
            let old_parent = self.entries[id as usize].parent;
            if old_parent != parent || was_dead {
                self.detach(old_parent, id);
                self.entries[id as usize].parent = parent;
                if parent != NO_PARENT {
                    self.link_child(parent, id);
                }
            }
            self.set_name(id, name);
            self.entries[id as usize].flags = flags & !TOMBSTONE;
            if flags & DIRECTORY == 0 {
                self.set_size_at(id as usize, size);
            }
            if modified > 0 {
                self.entries[id as usize].modified = modified.min(u64::from(u32::MAX)) as u32;
            }
            self.add_trigrams(id);
            self.sizes_valid = false;
        } else {
            let file_ref = if file_ref == 0 {
                self.next_file_ref
            } else {
                file_ref
            };
            self.alloc(
                volume,
                parent,
                file_ref,
                name,
                size,
                modified,
                flags & !TOMBSTONE,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn alloc(
        &mut self,
        volume: u16,
        parent: u32,
        file_ref: u64,
        name: &str,
        size: u64,
        modified: u64,
        flags: u16,
    ) -> u32 {
        let id = self.entries.len() as u32;
        let (name_start, name_len) = push_str(&mut self.names, name);
        let size_lo = if size > u64::from(u32::MAX) {
            self.big_sizes.insert(id, size);
            u32::MAX
        } else {
            size as u32
        };
        self.file_refs.push(file_ref);
        self.entries.push(Entry {
            parent,
            name_start,
            first_child: NO_PARENT,
            next_sibling: NO_PARENT,
            volume,
            flags: flags & !TOMBSTONE,
            name_len,
            _pad: 0,
            size_lo,
            modified: modified.min(u64::from(u32::MAX)) as u32,
        });
        if parent != NO_PARENT && (parent as usize) < self.entries.len() - 1 {
            self.link_child(parent, id);
        }
        if !(file_ref == u64::from(id) + 1 && self.file_refs.len() == self.entries.len()) {
            self.refs.insert(volume, file_ref, id);
        }
        self.add_trigrams(id);
        if file_ref >= self.next_file_ref {
            self.next_file_ref = file_ref.saturating_add(1);
        }
        self.sizes_valid = false;
        id
    }

    fn tombstone(&mut self, id: u32) {
        if !self.contains(id) || self.entries[id as usize].flags & TOMBSTONE != 0 {
            return;
        }
        self.clear_trigrams(id);
        let parent = self.entries[id as usize].parent;
        self.detach(parent, id);
        self.entries[id as usize].flags |= TOMBSTONE;
        self.sizes_valid = false;
    }

    fn tombstone_tree(&mut self, id: u32) {
        let mut stack = vec![id];
        let mut order = Vec::new();
        while let Some(current) = stack.pop() {
            if !self.contains(current) {
                continue;
            }
            order.push(current);
            stack.extend(self.child_ids(current));
        }
        for current in order.into_iter().rev() {
            self.tombstone(current);
        }
    }

    fn detach(&mut self, parent: u32, child: u32) {
        self.unlink_child(parent, child);
    }

    fn lookup_ref(&self, volume: u16, file_ref: u64) -> Option<u32> {
        if let Some(id) = self.refs.get(volume, file_ref) {
            return Some(id);
        }
        if file_ref == 0 {
            return None;
        }
        let id = (file_ref - 1) as u32;
        if (id as usize) < self.entries.len()
            && self.file_ref_at(id) == file_ref
            && self.entries[id as usize].volume == volume
        {
            return Some(id);
        }
        None
    }

    fn remember_ref(&mut self, volume: u16, file_ref: u64, id: u32) {
        self.refs.insert(volume, file_ref, id);
    }

    pub(crate) fn file_ref_at(&self, id: u32) -> u64 {
        self.file_refs
            .get(id as usize)
            .copied()
            .unwrap_or(u64::from(id) + 1)
    }

    pub(crate) fn size_at(&self, id: usize) -> u64 {
        self.big_sizes
            .get(&(id as u32))
            .copied()
            .unwrap_or(u64::from(self.entries[id].size_lo))
    }

    fn set_size_at(&mut self, id: usize, size: u64) {
        if size > u64::from(u32::MAX) {
            self.big_sizes.insert(id as u32, size);
            self.entries[id].size_lo = u32::MAX;
        } else {
            self.big_sizes.remove(&(id as u32));
            self.entries[id].size_lo = size as u32;
        }
    }

    fn child_ids(&self, parent: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let Some(entry) = self.entries.get(parent as usize) else {
            return out;
        };
        let mut child = entry.first_child;
        let mut guard = 0u32;
        while child != NO_PARENT && (child as usize) < self.entries.len() && guard < 1_000_000 {
            out.push(child);
            child = self.entries[child as usize].next_sibling;
            guard += 1;
        }
        out
    }

    fn link_child(&mut self, parent: u32, child: u32) {
        if parent == NO_PARENT || (parent as usize) >= self.entries.len() {
            return;
        }
        let next = self.entries[parent as usize].first_child;
        self.entries[child as usize].next_sibling = next;
        self.entries[parent as usize].first_child = child;
    }

    fn unlink_child(&mut self, parent: u32, child: u32) {
        if (parent as usize) >= self.entries.len() {
            return;
        }
        let mut prev = NO_PARENT;
        let mut current = self.entries[parent as usize].first_child;
        let mut guard = 0u32;
        while current != NO_PARENT && guard < 1_000_000 {
            let next = self.entries[current as usize].next_sibling;
            if current == child {
                if prev == NO_PARENT {
                    self.entries[parent as usize].first_child = next;
                } else {
                    self.entries[prev as usize].next_sibling = next;
                }
                self.entries[child as usize].next_sibling = NO_PARENT;
                return;
            }
            prev = current;
            current = next;
            guard += 1;
        }
    }

    fn push_entry(&mut self, volume: u16, file: &RawFile, parent: u32) {
        let (name_start, name_len) = push_str(&mut self.names, &file.name);
        let size_lo = if file.size > u64::from(u32::MAX) {
            u32::MAX
        } else {
            file.size as u32
        };
        let id = self.entries.len() as u32;
        if file.size > u64::from(u32::MAX) {
            self.big_sizes.insert(id, file.size);
        }
        self.file_refs.push(file.file_ref);
        self.entries.push(Entry {
            parent,
            name_start,
            first_child: NO_PARENT,
            next_sibling: NO_PARENT,
            volume,
            flags: file.flags,
            name_len,
            _pad: 0,
            size_lo,
            modified: file.modified.min(u64::from(u32::MAX)) as u32,
        });
    }

    pub(crate) fn rebuild_derived(&mut self) {
        let count = self.entries.len();
        self.refs.clear();
        self.trigrams.clear();
        self.trigrams_ready = false;
        for entry in &mut self.entries {
            entry.first_child = NO_PARENT;
            entry.next_sibling = NO_PARENT;
        }
        let identity = self.file_refs.len() == count
            && self
                .file_refs
                .iter()
                .enumerate()
                .all(|(index, file_ref)| *file_ref == index as u64 + 1);
        for index in 0..count {
            if self.entries[index].flags & TOMBSTONE != 0 {
                continue;
            }
            let volume = self.entries[index].volume;
            let file_ref = self.file_ref_at(index as u32);
            let parent = self.entries[index].parent;
            if !identity {
                self.refs.insert(volume, file_ref, index as u32);
            }
            if parent != NO_PARENT
                && (parent as usize) < count
                && parent as usize != index
                && self.entries[parent as usize].flags & TOMBSTONE == 0
            {
                self.link_child(parent, index as u32);
            }
        }
        if identity {
            self.file_refs.clear();
            self.file_refs.shrink_to_fit();
        }
        self.entries.shrink_to_fit();
        self.names.shrink_to_fit();
        self.sizes_valid = false;
        self.sizes.clear();
    }

    fn consider(
        &self,
        heap: &mut BinaryHeap<Rank>,
        id: u32,
        needle: &str,
        dir_filter: Option<&str>,
        limit: usize,
    ) {
        if !self.alive(id) {
            return;
        }
        let name = self.name_of(id).to_lowercase();
        if name.is_empty() {
            return;
        }
        if !needle.is_empty() && !name.contains(needle) {
            return;
        }
        if let Some(dir) = dir_filter {
            if dir.is_empty() {
                return;
            }
            let path = self.path(id).to_lowercase();
            if !path.contains(dir) {
                return;
            }
        }
        let rank = Rank {
            score: score(&name, needle),
            len: name.len() as u32,
            name,
            id,
        };
        if heap.len() < limit {
            heap.push(rank);
        } else if let Some(worst) = heap.peek() {
            if &rank < worst {
                heap.pop();
                heap.push(rank);
            }
        }
    }

    fn hit(&self, id: u32) -> SearchHit {
        let entry = &self.entries[id as usize];
        SearchHit {
            id,
            name: self.name_of(id).to_string(),
            path: self.path(id),
            is_dir: entry.flags & DIRECTORY != 0,
            size: self.size_at(id as usize),
            modified: u64::from(entry.modified),
        }
    }

    fn child_named(&self, parent: u32, name: &str, insensitive: bool) -> Option<u32> {
        self.child_ids(parent)
            .into_iter()
            .find(|id| self.alive(*id) && names_equal(self.name_of(*id), name, insensitive))
    }

    fn best_volume(&self, path: &str) -> Option<(usize, bool)> {
        let mut best: Option<(usize, usize)> = None;
        for (index, volume) in self.volumes.iter().enumerate() {
            let path_n = normalize_path(path, &volume.mount);
            let mount_n = normalize_path(&volume.mount, &volume.mount);
            if prefix_match(&path_n, &mount_n, volume.case_insensitive) {
                let score = trim_trailing(&mount_n).len();
                if best.map(|(_, len)| score >= len).unwrap_or(true) {
                    best = Some((index, score));
                }
            }
        }
        best.map(|(index, _)| (index, self.volumes[index].case_insensitive))
    }

    fn alive(&self, id: u32) -> bool {
        self.contains(id) && self.entries[id as usize].flags & TOMBSTONE == 0
    }

    fn contains(&self, id: u32) -> bool {
        (id as usize) < self.entries.len()
    }

    fn is_dir(&self, id: u32) -> bool {
        self.alive(id) && self.entries[id as usize].flags & DIRECTORY != 0
    }

    fn text(&self, id: u32) -> &str {
        let entry = &self.entries[id as usize];
        let start = entry.name_start as usize;
        let len = entry.name_len as usize;
        let end = start.saturating_add(len).min(self.names.len());
        std::str::from_utf8(self.names.get(start..end).unwrap_or_default()).unwrap_or("")
    }

    fn set_name(&mut self, id: u32, name: &str) {
        let (name_start, name_len) = push_str(&mut self.names, name);
        let entry = &mut self.entries[id as usize];
        entry.name_start = name_start;
        entry.name_len = name_len;
    }

    fn clear_trigrams(&mut self, id: u32) {
        if !self.trigrams_ready {
            return;
        }
        let folded = self.name_of(id).to_lowercase();
        let bytes = folded.as_bytes();
        for gram in unique_trigrams(bytes) {
            if let Some(list) = self.trigrams.get_mut(&gram) {
                if let Ok(position) = list.binary_search(&id) {
                    list.remove(position);
                }
            }
        }
    }

    fn add_trigrams(&mut self, id: u32) {
        if !self.trigrams_ready {
            return;
        }
        let folded = self.name_of(id).to_lowercase();
        let bytes = folded.as_bytes();
        for gram in unique_trigrams(bytes) {
            let list = self.trigrams.entry(gram).or_default();
            if let Err(position) = list.binary_search(&id) {
                list.insert(position, id);
            }
        }
    }

    fn ensure_trigrams(&mut self) {
        if self.trigrams_ready {
            return;
        }
        self.trigrams.clear();
        let count = self.entries.len();
        for index in 0..count {
            if self.entries[index].flags & TOMBSTONE == 0 {
                self.add_trigrams_bulk(index as u32);
            }
        }
        for list in self.trigrams.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
        self.trigrams_ready = true;
    }

    fn add_trigrams_bulk(&mut self, id: u32) {
        let folded = self.name_of(id).to_lowercase();
        for gram in unique_trigrams(folded.as_bytes()) {
            self.trigrams.entry(gram).or_default().push(id);
        }
    }
}

#[derive(Eq, PartialEq)]
struct Rank {
    score: u8,
    len: u32,
    name: String,
    id: u32,
}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .cmp(&other.score)
            .then(self.len.cmp(&other.len))
            .then(self.name.cmp(&other.name))
            .then(self.id.cmp(&other.id))
    }
}

fn score(name: &str, needle: &str) -> u8 {
    if needle.is_empty() {
        2
    } else if name == needle {
        0
    } else if name.starts_with(needle) {
        1
    } else {
        2
    }
}

fn split_query(query: &str) -> (Option<String>, String) {
    match query.rfind(['/', '\\']) {
        Some(index) => (
            Some(query[..index].to_string()),
            query[index + 1..].to_string(),
        ),
        None => (None, query.to_string()),
    }
}

fn trigram_key(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn unique_trigrams(bytes: &[u8]) -> Vec<u32> {
    if bytes.len() < 3 {
        return Vec::new();
    }
    let mut keys: Vec<u32> = bytes.windows(3).map(trigram_key).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn intersect(lists: &[&[u32]]) -> Vec<u32> {
    if lists.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<&[u32]> = lists.to_vec();
    order.sort_by_key(|list| list.len());
    let mut acc = order[0].to_vec();
    for list in order.iter().skip(1) {
        acc = intersect_two(&acc, list);
        if acc.is_empty() {
            break;
        }
    }
    acc
}

fn intersect_two(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut i = 0;
    let mut j = 0;
    let mut out = Vec::new();
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            Ordering::Equal => {
                out.push(left[i]);
                i += 1;
                j += 1;
            }
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
        }
    }
    out
}

fn push_str(buf: &mut Vec<u8>, text: &str) -> (u32, u16) {
    let start = buf.len() as u32;
    let bytes = text.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    buf.extend_from_slice(&bytes[..len]);
    (start, len as u16)
}

fn names_equal(left: &str, right: &str, insensitive: bool) -> bool {
    if insensitive {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

fn separator(mount: &str) -> char {
    if mount.contains('\\') || (mount.len() >= 2 && mount.as_bytes()[1] == b':') {
        '\\'
    } else {
        '/'
    }
}

fn normalize_path(path: &str, mount: &str) -> String {
    if separator(mount) == '\\' {
        path.replace('/', "\\")
    } else {
        path.replace('\\', "/")
    }
}

fn trim_trailing(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        path
    } else {
        trimmed
    }
}

fn prefix_match(path: &str, mount: &str, insensitive: bool) -> bool {
    let mount_t = trim_trailing(mount);
    let path_t = trim_trailing(path);
    if mount_t == "/" {
        return path.starts_with('/');
    }
    let same = |left: &str, right: &str| {
        if insensitive {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    };
    if same(path_t, mount_t) {
        return true;
    }
    let sep = if mount.contains('\\') { '\\' } else { '/' };
    let with_sep = format!("{mount_t}{sep}");
    if insensitive {
        path.len() >= with_sep.len() && path[..with_sep.len()].eq_ignore_ascii_case(&with_sep)
    } else {
        path.starts_with(&with_sep)
    }
}

fn remainder(path: &str, mount: &str, insensitive: bool) -> String {
    let path_n = normalize_path(path, mount);
    let mount_n = normalize_path(mount, mount);
    let mount_t = trim_trailing(&mount_n);
    let path_t = trim_trailing(&path_n);
    let same = |left: &str, right: &str| {
        if insensitive {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    };
    if mount_t == "/" {
        return path_t.trim_start_matches('/').to_string();
    }
    if same(path_t, mount_t) {
        return String::new();
    }
    let sep = separator(mount);
    let with_sep = format!("{mount_t}{sep}");
    if path_n.len() >= with_sep.len()
        && ((insensitive && path_n[..with_sep.len()].eq_ignore_ascii_case(&with_sep))
            || (!insensitive && path_n.starts_with(&with_sep)))
    {
        path_n[with_sep.len()..].to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::{DIRECTORY, HIDDEN};
    use crate::usn::{
        USN_REASON_CLOSE, USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE,
        USN_REASON_RENAME_NEW_NAME, USN_REASON_RENAME_OLD_NAME,
    };

    fn raw(file_ref: u64, parent: u64, name: &str, size: u64, flags: u16) -> RawFile {
        RawFile {
            file_ref,
            parent_ref: parent,
            name: name.into(),
            size,
            modified: 10,
            flags,
        }
    }

    fn sample() -> Index {
        let mut builder = IndexBuilder::new();
        let volume = builder.add_volume("/data".into(), "Data".into(), false);
        builder.add_file(volume, raw(1, 1, "", 0, DIRECTORY));
        builder.add_file(volume, raw(2, 1, "Photos", 0, DIRECTORY));
        builder.add_file(volume, raw(3, 2, "Annual Report.pdf", 50, 0));
        builder.add_file(volume, raw(4, 2, "notes.txt", 4, 0));
        builder.add_file(volume, raw(5, 1, "ab", 1, 0));
        builder.add_file(volume, raw(6, 1, "abc", 1, 0));
        builder.add_file(volume, raw(7, 1, ".secret", 2, HIDDEN));
        builder.finish()
    }

    #[test]
    fn entry_record_stays_within_32_bytes() {
        assert!(
            std::mem::size_of::<Entry>() <= 32,
            "{}",
            std::mem::size_of::<Entry>()
        );
    }

    #[test]
    fn search_ranks_exact_then_prefix_then_substring() {
        let mut index = sample();
        let hits = index.search("report", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Annual Report.pdf");
        assert_eq!(hits[0].path, "/data/Photos/Annual Report.pdf");

        let ranked = index.search("ab", 10);
        assert_eq!(ranked[0].name, "ab");
        assert_eq!(ranked[1].name, "abc");
        assert!(index.search("no-such-file-xyz", 5).is_empty());
        assert!(index.search("   ", 5).is_empty());
    }

    #[test]
    fn path_round_trip_and_folder_size() {
        let mut index = sample();
        let photos = index.find_path("/data/Photos").unwrap();
        assert_eq!(index.name_of(photos), "Photos");
        let root = index.find_path("/data").unwrap();
        index.ensure_sizes();
        assert_eq!(index.folder_size(root), 50 + 4 + 1 + 1 + 2);
        let visible = index.list_dir(root, false);
        assert!(visible.iter().all(|row| row.name != ".secret"));
        assert!(index
            .list_dir(root, true)
            .iter()
            .any(|row| row.name == ".secret"));
    }

    #[test]
    fn usn_create_rename_and_delete() {
        let mut index = sample();
        let volume = 0u16;
        index.apply_usn(
            volume,
            &UsnRecord {
                file_ref: 8,
                parent_ref: 1,
                usn: 1,
                timestamp: 20,
                reason: USN_REASON_FILE_CREATE | USN_REASON_CLOSE,
                attributes: 0,
                name: "new.txt".into(),
            },
        );
        assert_eq!(index.search("new.txt", 5).len(), 1);
        index.apply_usn(
            volume,
            &UsnRecord {
                file_ref: 8,
                parent_ref: 1,
                usn: 2,
                timestamp: 21,
                reason: USN_REASON_RENAME_OLD_NAME,
                attributes: 0,
                name: "new.txt".into(),
            },
        );
        index.apply_usn(
            volume,
            &UsnRecord {
                file_ref: 8,
                parent_ref: 2,
                usn: 3,
                timestamp: 22,
                reason: USN_REASON_RENAME_NEW_NAME | USN_REASON_CLOSE,
                attributes: 0,
                name: "moved.txt".into(),
            },
        );
        assert!(index.search("new.txt", 5).is_empty());
        let moved = index.search("moved.txt", 5);
        assert_eq!(moved.len(), 1);
        assert!(moved[0].path.ends_with("/Photos/moved.txt"));
        index.apply_usn(
            volume,
            &UsnRecord {
                file_ref: 4,
                parent_ref: 2,
                usn: 4,
                timestamp: 23,
                reason: USN_REASON_FILE_DELETE | USN_REASON_CLOSE,
                attributes: 0,
                name: "notes.txt".into(),
            },
        );
        assert!(index.search("notes.txt", 5).is_empty());
    }

    #[test]
    fn reconcile_adds_and_removes_children() {
        let mut index = sample();
        let root = index.find_path("/data").unwrap();
        let created = index.reconcile_children(
            root,
            &[
                LiveChild {
                    name: "Photos".into(),
                    is_dir: true,
                    size: 0,
                    modified: 1,
                    flags: DIRECTORY,
                },
                LiveChild {
                    name: "fresh.txt".into(),
                    is_dir: false,
                    size: 9,
                    modified: 2,
                    flags: 0,
                },
            ],
        );
        assert!(created.is_empty());
        assert!(index.search("fresh.txt", 5).iter().any(|hit| hit.size == 9));
        assert_eq!(index.search("notes.txt", 5).len(), 1);
        assert!(index.search("ab", 5).is_empty());
    }

    #[test]
    fn windows_style_path_is_case_insensitive() {
        let mut builder = IndexBuilder::new();
        let volume = builder.add_volume("C:\\".into(), "OS".into(), true);
        builder.add_file(volume, raw(1, 1, "", 0, DIRECTORY));
        builder.add_file(volume, raw(2, 1, "Users", 0, DIRECTORY));
        builder.add_file(volume, raw(3, 2, "Readme.md", 8, 0));
        let index = builder.finish();
        let id = index.find_path("c:/users/readme.md").unwrap();
        assert_eq!(index.path(id), "C:\\Users\\Readme.md");
    }

    #[test]
    fn subtree_items_counts_the_folder_and_its_descendants() {
        let index = sample();
        let root = index.find_path("/data").unwrap();
        let notes = index.find_path("/data/Photos/notes.txt").unwrap();
        assert_eq!(index.subtree_items(notes), 1);
        assert!(index.subtree_items(root) >= 4);
    }

    #[test]
    fn index_stays_compact_before_the_first_search() {
        let mut builder = IndexBuilder::new();
        let volume = builder.add_volume("/data".into(), "data".into(), false);
        builder.add_file(volume, raw(1, 1, "", 0, DIRECTORY));
        for index in 0..20_000 {
            let dir = 2 + (index % 50);
            if index < 50 {
                builder.add_file(volume, raw(dir, 1, &format!("d{dir}"), 0, DIRECTORY));
            }
            builder.add_file(
                volume,
                raw(1_000 + index, dir, &format!("file-{index}.txt"), 128, 0),
            );
        }
        let index = builder.finish();
        let count = index.entry_count().max(1);
        let bytes = index.resident_bytes();
        let per = bytes / count;
        eprintln!("index entries={count} resident_bytes={bytes} bytes_per_entry={per}");
        assert!(
            per < 100,
            "index uses {per} bytes/entry ({bytes} for {count})"
        );
    }
}
