//! File operations that do not need a window: copy, move, rename, trash.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct Clipboard {
    pub cut: bool,
    pub paths: Vec<PathBuf>,
}

pub fn copy_into(src: &Path, dest_dir: &Path) -> Result<PathBuf> {
    if path_is_inside(src, dest_dir) {
        return Err(Error::new("Cannot copy a folder into itself"));
    }
    let name = file_name(src)?;
    let dest_name = unique_name(dest_dir, &name);
    let dest = dest_dir.join(dest_name);
    copy_recursive(src, &dest)?;
    Ok(dest)
}

pub fn paste(clip: &Clipboard, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for src in &clip.paths {
        if !src.exists() {
            return Err(Error::new(format!("Missing {}", src.display())));
        }
        if clip.cut {
            if path_is_inside(src, dest_dir) {
                return Err(Error::new("Cannot move a folder into itself"));
            }
            if same_dir(src.parent(), Some(dest_dir)) {
                written.push(src.clone());
                continue;
            }
            let dest_name = unique_name(dest_dir, &file_name(src)?);
            let dest = dest_dir.join(dest_name);
            match fs::rename(src, &dest) {
                Ok(()) => written.push(dest),
                Err(_) => {
                    copy_recursive(src, &dest)?;
                    remove_permanently(src)?;
                    written.push(dest);
                }
            }
        } else {
            written.push(copy_into(src, dest_dir)?);
        }
    }
    Ok(written)
}

pub fn rename_path(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_name(new_name)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("Cannot rename this path"))?;
    let dest = parent.join(new_name);
    if dest.exists() {
        return Err(Error::new("A file with that name already exists"));
    }
    fs::rename(path, &dest)?;
    Ok(dest)
}

pub fn create_folder(dir: &Path) -> Result<PathBuf> {
    create_directory(dir, "New folder")
}

pub fn create_directory(dir: &Path, base: &str) -> Result<PathBuf> {
    let name = unique_name(dir, base);
    let path = dir.join(name);
    fs::create_dir(&path)?;
    Ok(path)
}

pub fn create_text_file(dir: &Path, base: &str) -> Result<PathBuf> {
    let name = unique_name(dir, base);
    let path = dir.join(name);
    fs::write(&path, b"")?;
    Ok(path)
}

pub fn delete_to_trash(path: &Path) -> Result<()> {
    trash::delete(path)?;
    Ok(())
}

pub fn remove_permanently(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        fs::remove_file(path)?;
    } else {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub fn unique_name(dir: &Path, file_name: &str) -> String {
    if !dir.join(file_name).exists() {
        return file_name.to_string();
    }
    let (stem, ext) = split_name(file_name);
    for index in 2..10_000 {
        let candidate = match ext {
            Some(ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{stem} ({index})"),
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem}-{}", std::process::id())
}

fn copy_recursive(src: &Path, dest: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return copy_symlink(src, dest);
    }
    if meta.is_dir() {
        fs::create_dir(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        copy_file(src, dest)?;
        Ok(())
    }
}

fn copy_file(src: &Path, dest: &Path) -> io::Result<u64> {
    let mut from = File::open(src)?;
    let mut to = File::create(dest)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = from.read(&mut buf)?;
        if read == 0 {
            break;
        }
        to.write_all(&buf[..read])?;
        total += read as u64;
    }
    Ok(total)
}

fn copy_symlink(src: &Path, dest: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let target = fs::read_link(src)?;
        std::os::unix::fs::symlink(target, dest)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = src;
        let _ = dest;
        fs::copy(src, dest)?;
        Ok(())
    }
}

fn path_is_inside(dir: &Path, path: &Path) -> bool {
    match (canonicalize_existing(dir), canonicalize_existing(path)) {
        (Some(dir), Some(path)) => path.starts_with(&dir) && path != dir,
        _ => false,
    }
}

fn canonicalize_existing(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn same_dir(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => match (fs::canonicalize(a), fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == b,
        },
        _ => false,
    }
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| Error::new("Missing file name"))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(Error::new("That name cannot be used"));
    }
    Ok(())
}

fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn copy_rename_and_collision() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        let dest_dir = dir.path().join("dest");
        fs::create_dir_all(src_dir.join("nested")).unwrap();
        fs::create_dir_all(&dest_dir).unwrap();
        let file = src_dir.join("nested").join("note.txt");
        fs::File::create(&file).unwrap().write_all(b"abc").unwrap();
        fs::write(dest_dir.join("note.txt"), b"old").unwrap();

        let copied = copy_into(&src_dir.join("nested"), &dest_dir).unwrap();
        assert_eq!(fs::read(copied.join("note.txt")).unwrap(), b"abc");

        let again = copy_into(&file, &dest_dir).unwrap();
        assert_eq!(again.file_name().unwrap(), "note (2).txt");

        let renamed = rename_path(&again, "renamed.txt").unwrap();
        assert!(renamed.ends_with("renamed.txt"));
        assert!(rename_path(&renamed, "a/b").is_err());

        let created = create_folder(&dest_dir).unwrap();
        assert_eq!(created.file_name().unwrap(), "New folder");
        let second = create_folder(&dest_dir).unwrap();
        assert_eq!(second.file_name().unwrap(), "New folder (2)");
    }

    #[test]
    fn refuses_to_copy_a_directory_into_itself() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let inner = folder.join("inner");
        fs::create_dir(&inner).unwrap();
        assert!(copy_into(&folder, &inner).is_err());
    }

    #[test]
    fn cut_paste_moves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let file = dir.path().join("move-me.txt");
        fs::write(&file, b"z").unwrap();
        let clip = Clipboard {
            cut: true,
            paths: vec![file.clone()],
        };
        let written = paste(&clip, &dest).unwrap();
        assert!(written[0].ends_with("move-me.txt"));
        assert!(!file.exists());
        assert_eq!(fs::read(&written[0]).unwrap(), b"z");
    }

    #[test]
    fn creates_a_folder_and_a_text_file_with_the_given_names() {
        let dir = tempfile::tempdir().unwrap();
        let folder = create_directory(dir.path(), "Nouveau dossier").unwrap();
        assert!(folder.is_dir());
        let file = create_text_file(dir.path(), "Nouveau document texte.txt").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"");
        let again = create_text_file(dir.path(), "Nouveau document texte.txt").unwrap();
        assert_eq!(again.file_name().unwrap(), "Nouveau document texte (2).txt");
    }

    #[test]
    fn trash_removes_the_path_from_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gone.txt");
        fs::write(&file, b"trash").unwrap();
        delete_to_trash(&file).expect("trash");
        assert!(!file.exists());
    }
}
