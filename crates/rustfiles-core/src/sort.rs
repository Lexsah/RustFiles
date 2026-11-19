use std::cmp::Ordering;

use crate::model::FileRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Modified,
    Type,
    Size,
}

/// ASCII natural order: digit runs compare as integers, letters ignore case.
/// A leading-zero difference is a tie-break used only when the rest of the name matches.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut i = 0;
    let mut j = 0;
    let mut zero_tie = Ordering::Equal;
    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            let i_start = i;
            let j_start = j;
            while i < ab.len() && ab[i] == b'0' {
                i += 1;
            }
            while j < bb.len() && bb[j] == b'0' {
                j += 1;
            }
            let i_sig = i;
            let j_sig = j;
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let asig = &ab[i_sig..i];
            let bsig = &bb[j_sig..j];
            match asig.len().cmp(&bsig.len()).then(asig.cmp(bsig)) {
                Ordering::Equal => {
                    if zero_tie == Ordering::Equal {
                        zero_tie = (i_sig - i_start).cmp(&(j_sig - j_start));
                    }
                }
                other => return other,
            }
        } else {
            let ca = ab[i].to_ascii_lowercase();
            let cb = bb[j].to_ascii_lowercase();
            match ca.cmp(&cb) {
                Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
                other => return other,
            }
        }
    }
    if i == ab.len() && j == bb.len() {
        zero_tie
    } else if i == ab.len() {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

pub fn sort_rows(rows: &mut [FileRow], column: SortColumn, ascending: bool, dirs_first: bool) {
    rows.sort_by(|a, b| {
        if dirs_first {
            match (a.is_dir, b.is_dir) {
                (true, false) => return Ordering::Less,
                (false, true) => return Ordering::Greater,
                _ => {}
            }
        }
        let mut ord = match column {
            SortColumn::Name => natural_cmp(&a.name, &b.name),
            SortColumn::Size => a.size.cmp(&b.size).then(natural_cmp(&a.name, &b.name)),
            SortColumn::Modified => a
                .modified
                .cmp(&b.modified)
                .then(natural_cmp(&a.name, &b.name)),
            SortColumn::Type => type_label(a)
                .cmp(&type_label(b))
                .then(natural_cmp(&a.name, &b.name)),
        };
        if !ascending {
            ord = ord.reverse();
        }
        ord
    });
}

fn type_label(row: &FileRow) -> String {
    if row.is_dir {
        return "folder".to_string();
    }
    match row.name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_compare_numerically_and_case_is_ignored() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("file10", "file2"), Ordering::Greater);
        assert_eq!(natural_cmp("Img", "img"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "b"), Ordering::Less);
        assert_eq!(natural_cmp("file01", "file1"), Ordering::Greater);
        assert_eq!(natural_cmp("file1", "file01"), Ordering::Less);
    }

    #[test]
    fn directories_stay_above_files_when_requested() {
        let mut rows = vec![row("b.txt", false, 5), row("a", true, 1), row("c", true, 9)];
        sort_rows(&mut rows, SortColumn::Name, true, true);
        assert!(rows[0].is_dir && rows[1].is_dir);
        assert_eq!(rows[0].name, "a");
        assert_eq!(rows[2].name, "b.txt");
    }

    fn row(name: &str, is_dir: bool, size: u64) -> FileRow {
        FileRow {
            name: name.into(),
            path: name.into(),
            is_dir,
            hidden: false,
            size,
            modified: 0,
        }
    }
}
