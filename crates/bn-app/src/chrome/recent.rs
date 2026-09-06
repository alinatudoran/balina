//! Recent-files list behind File > Open Recent: an in-process copy for menu
//! routing plus a plain one-path-per-line file under the user config dir so
//! it survives restarts. All calls run on the UI thread (menu routing and
//! file ops), so a Mutex is only defensive.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAX_RECENT: usize = 8;

static RECENTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn store_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("balina").join("recent_files.txt"))
}

/// Load from disk (menu build time), dropping entries that no longer exist.
pub fn load() -> Vec<PathBuf> {
    let list: Vec<PathBuf> = store_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .take(MAX_RECENT)
        .collect();
    *RECENTS.lock().unwrap() = list.clone();
    list
}

pub fn get(index: usize) -> Option<PathBuf> {
    RECENTS.lock().unwrap().get(index).cloned()
}

/// Move `path` to the front, then sync the disk file and the native submenu.
pub fn add(path: &Path) {
    let list = {
        let mut l = RECENTS.lock().unwrap();
        l.retain(|p| p != path);
        l.insert(0, path.to_path_buf());
        l.truncate(MAX_RECENT);
        l.clone()
    };
    persist(&list);
    crate::chrome::menu::rebuild_recent_items(&list);
}

pub fn clear() {
    RECENTS.lock().unwrap().clear();
    persist(&[]);
    crate::chrome::menu::rebuild_recent_items(&[]);
}

fn persist(list: &[PathBuf]) {
    let Some(p) = store_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::write(&p, text);
}
