//! File ops. Native file pickers live in the UI layer (rfd); these take
//! plain paths so they stay headless.

use std::path::PathBuf;

use bn_core::io;

use crate::doc::{Dirt, Document};
use crate::error::CmdError;
use crate::session::{Session, Tab};
use crate::views::SaveResult;

pub fn doc_new(s: &mut Session) {
    s.reset();
}

pub fn doc_open(s: &mut Session, path: PathBuf) -> Result<(), CmdError> {
    let fmt = io::Format::from_path(&path)?;
    let text = std::fs::read_to_string(&path)?;
    doc_open_str(s, &text, fmt)?;
    s.path = Some(path);
    Ok(())
}

/// Open from already-loaded text (web builds — there is no path to keep).
pub fn doc_open_str(s: &mut Session, text: &str, fmt: io::Format) -> Result<(), CmdError> {
    match fmt {
        io::Format::NativeJson => {
            let project = io::load_project_str(text)?;
            let tabs: Vec<Tab> = project
                .sheets
                .into_iter()
                .map(|sheet| {
                    let mut tab = Tab::new();
                    tab.doc = Document::from_io_document(sheet.doc, None);
                    tab.bridge.invalidate();
                    tab.bridge.recompute(&tab.doc);
                    tab
                })
                .collect();
            if tabs.is_empty() {
                return Err(CmdError::BadRequest("project contains no tabs".into()));
            }
            s.load_tabs(tabs, project.active);
        }
        _ => {
            // XMLBIF / XDSL: single-network import → one tab
            let iodoc = io::load_str(text, fmt)?;
            let mut tab = Tab::new();
            tab.doc = Document::from_io_document(iodoc, None);
            tab.bridge.invalidate();
            tab.bridge.recompute(&tab.doc);
            s.load_tabs(vec![tab], 0);
        }
    }
    s.path = None;
    s.modified = false;
    Ok(())
}

/// Serialize for a platform-side save (web builds: the caller downloads the
/// text). Clears `modified` like a successful save; `s.path` is untouched.
pub fn doc_save_str(s: &mut Session, fmt: io::Format) -> Result<(String, Vec<String>), CmdError> {
    match fmt {
        io::Format::NativeJson => {
            let project = session_to_project(s);
            let text = io::save_project_str(&project)?;
            clear_modified(s);
            Ok((text, vec![]))
        }
        _ => {
            // XMLBIF / XDSL: export active tab only
            let mut warnings = Vec::new();
            if s.tab_count() > 1 {
                warnings.push("only the active tab was exported (format does not support multiple networks)".into());
            }
            let (text, io_warnings) = io::save_str(&s.doc().to_io_document(), fmt)?;
            warnings.extend(io_warnings.into_iter().map(|io::Warning::Lossy(m)| m));
            clear_modified(s);
            Ok((text, warnings))
        }
    }
}

/// `path: None` saves to the project's current path (error if it has none —
/// the UI runs a save dialog first in that case).
pub fn doc_save(s: &mut Session, path: Option<PathBuf>) -> Result<SaveResult, CmdError> {
    let path = match path {
        Some(p) => p,
        None => s
            .path
            .clone()
            .ok_or_else(|| CmdError::BadRequest("project has no path; use Save As".into()))?,
    };
    let fmt = io::Format::from_path(&path)?;
    match fmt {
        io::Format::NativeJson => {
            let project = session_to_project(s);
            io::save_project(&project, &path)?;
        }
        _ => {
            let warnings_io = io::save(&s.doc().to_io_document(), &path)?;
            if !warnings_io.is_empty() {
                // warnings are surfaced by the caller
            }
        }
    }
    s.path = Some(path.clone());
    clear_modified(s);
    Ok(SaveResult { path, warnings: vec![] })
}

/// Initial mount: make sure beliefs exist if possible.
pub fn refresh(s: &mut Session) {
    s.finish(Dirt::None);
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn session_to_project(s: &Session) -> io::Project {
    let active = s.tab_order().iter().position(|&id| id == s.active_id()).unwrap_or(0);
    let sheets = s
        .tab_order()
        .iter()
        .map(|&id| {
            let tab = s.tab(id);
            io::ProjectSheet {
                label: tab.doc.net.name.clone(),
                doc: tab.doc.to_io_document(),
            }
        })
        .collect();
    io::Project { sheets, active }
}

fn clear_modified(s: &mut Session) {
    let ids: Vec<_> = s.tab_order().to_vec();
    for id in ids {
        s.tab_mut(id).doc.modified = false;
    }
    s.modified = false;
}
