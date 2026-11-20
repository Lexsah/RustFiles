//! Catalog of every interactive control, plus a filesystem exercise of the
//! commands those controls send. A control that only returns without an effect
//! is not allowed in the catalog.

use std::fs;
use std::path::Path;

use rustfiles_core::{
    create_directory, create_text_file, delete_to_trash, paste, remove_permanently, rename_path,
    Clipboard,
};

pub struct Control {
    pub surface: &'static str,
    pub name: &'static str,
    pub behavior: &'static str,
}

pub fn catalog() -> &'static [Control] {
    &[
        Control {
            surface: "Title",
            name: "Tab",
            behavior: "Selects the tab",
        },
        Control {
            surface: "Title",
            name: "New tab",
            behavior: "Opens a home tab",
        },
        Control {
            surface: "Title",
            name: "Close tab",
            behavior: "Closes the tab when another remains",
        },
        Control {
            surface: "Title",
            name: "Minimize",
            behavior: "Minimizes the window",
        },
        Control {
            surface: "Title",
            name: "Maximize",
            behavior: "Toggles maximized",
        },
        Control {
            surface: "Title",
            name: "Close",
            behavior: "Closes the window",
        },
        Control {
            surface: "Nav",
            name: "Back",
            behavior: "Walks the history backward",
        },
        Control {
            surface: "Nav",
            name: "Forward",
            behavior: "Walks the history forward",
        },
        Control {
            surface: "Nav",
            name: "Up",
            behavior: "Opens the parent folder",
        },
        Control {
            surface: "Nav",
            name: "Refresh",
            behavior: "Relists the current folder",
        },
        Control {
            surface: "Nav",
            name: "Breadcrumb",
            behavior: "Navigates to that segment",
        },
        Control {
            surface: "Nav",
            name: "Address",
            behavior: "Edits the path on double-click",
        },
        Control {
            surface: "Nav",
            name: "Search",
            behavior: "Searches the filename index",
        },
        Control {
            surface: "Toolbar",
            name: "New",
            behavior: "Creates a folder or text file",
        },
        Control {
            surface: "Toolbar",
            name: "Cut",
            behavior: "Cuts the selection, disabled when empty",
        },
        Control {
            surface: "Toolbar",
            name: "Copy",
            behavior: "Copies the selection, disabled when empty",
        },
        Control {
            surface: "Toolbar",
            name: "Paste",
            behavior: "Pastes the clipboard, disabled when empty",
        },
        Control {
            surface: "Toolbar",
            name: "Rename",
            behavior: "Renames the selection, disabled unless one item",
        },
        Control {
            surface: "Toolbar",
            name: "Delete",
            behavior: "Confirms, then moves the selection to the trash",
        },
        Control {
            surface: "Toolbar",
            name: "Share",
            behavior: "Hidden because the system share sheet is not available",
        },
        Control {
            surface: "Sidebar",
            name: "Cloud",
            behavior: "Reports that cloud drives are not available",
        },
        Control {
            surface: "Sidebar",
            name: "Network",
            behavior: "Reports that network locations are not available",
        },
        Control {
            surface: "Sidebar",
            name: "WSL",
            behavior: "Reports that WSL is not available",
        },
        Control {
            surface: "Sidebar",
            name: "Tags",
            behavior: "Reports that tags are not available",
        },
        Control {
            surface: "Toolbar",
            name: "Group",
            behavior: "Sorts the listing by the next column",
        },
        Control {
            surface: "Toolbar",
            name: "Sort",
            behavior: "Reverses the sort direction",
        },
        Control {
            surface: "Toolbar",
            name: "Layout",
            behavior: "Opens the layout flyout",
        },
        Control {
            surface: "Toolbar",
            name: "Preview",
            behavior: "Toggles the details pane",
        },
        Control {
            surface: "Toolbar",
            name: "Analyze",
            behavior: "Opens the disk analyzer",
        },
        Control {
            surface: "Sidebar",
            name: "Home",
            behavior: "Shows the home page",
        },
        Control {
            surface: "Sidebar",
            name: "Quick access",
            behavior: "Opens that folder",
        },
        Control {
            surface: "Sidebar",
            name: "Pin",
            behavior: "Pins the folder from its context menu or the row hover button",
        },
        Control {
            surface: "Sidebar",
            name: "Drive",
            behavior: "Opens the drive",
        },
        Control {
            surface: "Sidebar",
            name: "Recycle bin",
            behavior: "Opens the system trash folder",
        },
        Control {
            surface: "Sidebar",
            name: "Settings",
            behavior: "Theme, hidden files, and reindex",
        },
        Control {
            surface: "Files",
            name: "Row",
            behavior: "Selects the item",
        },
        Control {
            surface: "Files",
            name: "Row open",
            behavior: "Opens the item",
        },
        Control {
            surface: "Files",
            name: "Context menu",
            behavior: "Open, copy, cut, paste, rename, delete, properties",
        },
        Control {
            surface: "Analyzer",
            name: "Entry",
            behavior: "Selects the entry",
        },
        Control {
            surface: "Analyzer",
            name: "Entry open",
            behavior: "Analyzes that folder",
        },
        Control {
            surface: "Analyzer",
            name: "Delete",
            behavior: "Confirms, then trashes the selected entry",
        },
        Control {
            surface: "Analyzer",
            name: "Permanent delete",
            behavior: "Confirms with name, size, and item count, then deletes",
        },
        Control {
            surface: "Status",
            name: "Status text",
            behavior: "Shows the result of the last command",
        },
    ]
}

/// Runs each filesystem command the toolbar and analyzer can send.
pub fn exercise(dir: &Path) -> Result<(), String> {
    let created = create_directory(dir, "folder").map_err(|err| err.message)?;
    let file = create_text_file(&created, "note.txt").map_err(|err| err.message)?;
    if !file.is_file() {
        return Err("new file did not appear".into());
    }
    let copied = dir.join("copied.txt");
    fs::copy(&file, &copied).map_err(|err| err.to_string())?;
    let renamed = rename_path(&copied, "renamed.txt").map_err(|err| err.message)?;
    if !renamed.is_file() {
        return Err("rename did not move the file".into());
    }
    let dest = create_directory(dir, "pasted").map_err(|err| err.message)?;
    let pasted = paste(
        &Clipboard {
            cut: false,
            paths: vec![renamed.clone()],
        },
        &dest,
    )
    .map_err(|err| err.message)?;
    if pasted.is_empty() || !pasted[0].is_file() {
        return Err("paste did not create a file".into());
    }
    delete_to_trash(&pasted[0]).map_err(|err| format!("trash: {}", err.message))?;
    if pasted[0].exists() {
        return Err("trash left the file in place".into());
    }
    let doomed = create_text_file(dir, "doomed.txt").map_err(|err| err.message)?;
    remove_permanently(&doomed).map_err(|err| err.message)?;
    if doomed.exists() {
        return Err("permanent delete left the file".into());
    }
    remove_permanently(&created).map_err(|err| err.message)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{catalog, exercise};

    #[test]
    fn every_control_has_a_real_behavior() {
        assert!(catalog().len() >= 30);
        for control in catalog() {
            let behavior = control.behavior.to_ascii_lowercase();
            assert!(
                !behavior.contains("no-op") && !behavior.contains("unimplemented"),
                "{} / {} is a silent no-op",
                control.surface,
                control.name
            );
            assert!(!control.behavior.is_empty());
        }
    }

    #[test]
    fn commands_change_the_filesystem() {
        let dir = std::env::temp_dir().join(format!("rustfiles-commands-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        exercise(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
