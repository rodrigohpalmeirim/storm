//! Directory listing for the remote folder browser.

use std::path::{Path, PathBuf};

use storm_proto::{DirEntry, DirListing, Error, Result};

/// Lists subdirectories of `start`. If `start` doesn't exist, falls back to its nearest
/// existing ancestor (so a not-yet-created download folder still opens somewhere useful).
pub fn list_dirs(start: &str) -> Result<DirListing> {
    let requested = std::path::absolute(if start.trim().is_empty() { "." } else { start })
        .map_err(|e| Error::bad_request(format!("invalid path {start:?}: {e}")))?;
    let dir = nearest_existing_dir(&requested)
        .ok_or_else(|| Error::not_found(format!("no existing directory at or above {start:?}")))?;

    let read = std::fs::read_dir(&dir).map_err(|e| Error::bad_request(format!("reading {}: {e}", dir.display())))?;
    let mut dirs: Vec<DirEntry> = read
        .filter_map(|e| e.ok())
        // `metadata` follows symlinks/junctions; unreadable entries are skipped.
        .filter(|e| std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false))
        .map(|e| DirEntry {
            name: e.file_name().to_string_lossy().into_owned(),
            path: e.path().to_string_lossy().into_owned(),
        })
        .collect();
    dirs.sort_by_key(|d| d.name.to_lowercase());

    Ok(DirListing {
        path: dir.to_string_lossy().into_owned(),
        parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
        dirs,
        roots: roots(),
        separator: std::path::MAIN_SEPARATOR.to_string(),
    })
}

fn nearest_existing_dir(p: &Path) -> Option<PathBuf> {
    p.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf)
}

#[cfg(windows)]
fn roots() -> Vec<String> {
    (b'A'..=b'Z').map(|c| format!("{}:\\", c as char)).filter(|r| Path::new(r).is_dir()).collect()
}

#[cfg(not(windows))]
fn roots() -> Vec<String> {
    vec!["/".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_dirs_sorted_and_falls_back_to_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("beta")).unwrap();
        std::fs::create_dir(tmp.path().join("Alpha")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), b"x").unwrap();

        let l = list_dirs(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(l.dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Alpha", "beta"]);
        assert!(l.parent.is_some());
        assert!(!l.roots.is_empty());

        let missing = tmp.path().join("nope").join("deeper");
        let l2 = list_dirs(missing.to_str().unwrap()).unwrap();
        assert_eq!(Path::new(&l2.path), tmp.path());
    }
}
