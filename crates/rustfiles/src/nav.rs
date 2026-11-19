//! Path breadcrumbs and keyboard shortcuts. Both are pure so they can be tested
//! without opening a window.

use std::path::{Component, Path, PathBuf};

use egui::{Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewTab,
    CloseTab,
    NextTab,
    Back,
    Forward,
    Up,
    FocusPath,
    FocusSearch,
    Refresh,
    Rename,
    Trash,
    DeletePermanent,
    Copy,
    Cut,
    Paste,
    SelectAll,
    NewFolder,
    TogglePreview,
    Analyzer,
    Help,
    Escape,
    Open,
}

pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                acc.push(component);
                out.push((
                    prefix.as_os_str().to_string_lossy().into_owned(),
                    acc.clone(),
                ));
            }
            Component::RootDir => {
                acc.push(component);
                if let Some(last) = out.last_mut() {
                    last.1 = acc.clone();
                } else {
                    out.push(("/".to_string(), acc.clone()));
                }
            }
            Component::Normal(part) => {
                acc.push(part);
                out.push((part.to_string_lossy().into_owned(), acc.clone()));
            }
            Component::CurDir | Component::ParentDir => {
                acc.push(component);
            }
        }
    }
    out
}

/// More specific shortcuts must be listed before the keys they contain.
pub fn shortcut(modifiers: Modifiers, key: Key) -> Option<Action> {
    const TABLE: &[(Modifiers, Key, Action)] = &[
        (
            Modifiers::COMMAND.plus(Modifiers::SHIFT),
            Key::N,
            Action::NewFolder,
        ),
        (Modifiers::COMMAND, Key::T, Action::NewTab),
        (Modifiers::COMMAND, Key::W, Action::CloseTab),
        (Modifiers::COMMAND, Key::Tab, Action::NextTab),
        (Modifiers::COMMAND, Key::L, Action::FocusPath),
        (Modifiers::COMMAND, Key::F, Action::FocusSearch),
        (Modifiers::COMMAND, Key::A, Action::SelectAll),
        (Modifiers::COMMAND, Key::C, Action::Copy),
        (Modifiers::COMMAND, Key::X, Action::Cut),
        (Modifiers::COMMAND, Key::V, Action::Paste),
        (Modifiers::ALT, Key::ArrowLeft, Action::Back),
        (Modifiers::ALT, Key::ArrowRight, Action::Forward),
        (Modifiers::ALT, Key::ArrowUp, Action::Up),
        (Modifiers::ALT, Key::P, Action::TogglePreview),
        (Modifiers::SHIFT, Key::Delete, Action::DeletePermanent),
        (Modifiers::NONE, Key::Delete, Action::Trash),
        (Modifiers::NONE, Key::F2, Action::Rename),
        (Modifiers::NONE, Key::F1, Action::Help),
        (Modifiers::NONE, Key::F5, Action::Refresh),
        (Modifiers::NONE, Key::Backspace, Action::Up),
        (Modifiers::NONE, Key::Enter, Action::Open),
        (Modifiers::NONE, Key::Escape, Action::Escape),
    ];
    TABLE
        .iter()
        .find(|(pattern, table_key, _)| *table_key == key && modifiers.matches_logically(*pattern))
        .map(|(_, _, action)| *action)
}

pub fn parent_dir(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_breadcrumbs_keep_the_root() {
        let crumbs = breadcrumbs(Path::new("/home/ada"));
        assert_eq!(crumbs[0].0, "/");
        assert_eq!(crumbs[1].0, "home");
        assert_eq!(crumbs[2].0, "ada");
        assert_eq!(crumbs[2].1, PathBuf::from("/home/ada"));
    }

    #[test]
    fn shortcuts_prefer_the_more_specific_binding() {
        let shift_delete = Modifiers::SHIFT;
        assert_eq!(
            shortcut(shift_delete, Key::Delete),
            Some(Action::DeletePermanent)
        );
        assert_eq!(shortcut(Modifiers::NONE, Key::Delete), Some(Action::Trash));
        assert_eq!(
            shortcut(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::N),
            Some(Action::NewFolder)
        );
        assert_eq!(shortcut(Modifiers::COMMAND, Key::T), Some(Action::NewTab));
        assert_eq!(shortcut(Modifiers::NONE, Key::F5), Some(Action::Refresh));
    }
}
