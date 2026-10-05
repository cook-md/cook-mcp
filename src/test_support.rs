//! Test-only helpers shared by the tool modules.
#![cfg(test)]

use std::path::Path;

/// A fresh, writable copy of `tests/fixtures/workspace`.
pub fn fixture_workspace() -> (tempfile::TempDir, crate::workspace::Workspace) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace"),
        dir.path(),
    );
    let ws = crate::workspace::Workspace::new(dir.path()).unwrap();
    (dir, ws)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}
