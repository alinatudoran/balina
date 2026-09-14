//! Tab management ops.

use crate::doc::Dirt;
use crate::error::CmdError;
use crate::session::{Session, Tab, TabId};

/// Add a new empty tab after the active tab and switch to it.
pub fn add_tab(s: &mut Session) -> TabId {
    let id = s.insert_tab(Tab::new());
    s.set_active(id);
    s.modified = true;
    id
}

/// Close a tab. If it is the last tab, a fresh empty one is created first.
pub fn close_tab(s: &mut Session, id: TabId) -> Result<(), CmdError> {
    if s.tab_count() == 1 {
        // Can't close the last tab — replace with empty.
        let new_id = s.insert_tab(Tab::new());
        s.set_active(new_id);
    }
    s.remove_tab(id);
    s.modified = true;
    Ok(())
}

/// Switch to a different tab.
pub fn switch_tab(s: &mut Session, id: TabId) -> Result<(), CmdError> {
    s.set_active(id);
    Ok(())
}

/// Rename a tab (sets the network name).
pub fn rename_tab(s: &mut Session, id: TabId, name: String) {
    s.tab_mut(id).doc.net.name = name;
    s.modified = true;
}

/// Move a tab to a new position in the tab order.
pub fn reorder_tab(s: &mut Session, id: TabId, new_index: usize) {
    s.reorder_tab(id, new_index);
    s.modified = true;
}

/// Duplicate a tab (clone the document, fresh bridge) and switch to it.
pub fn duplicate_tab(s: &mut Session, id: TabId) -> Result<TabId, CmdError> {
    let src = s.tab(id);
    let mut new_tab = Tab::new();
    new_tab.doc = src.doc.clone_for_duplicate();
    let name = format!("{} (Copy)", new_tab.doc.net.name);
    new_tab.doc.net.name = name;
    new_tab.bridge.invalidate();
    let new_id = s.insert_tab(new_tab);
    s.set_active(new_id);
    s.finish(Dirt::Structure);
    Ok(new_id)
}
