//! File ops. Native file pickers live in the UI layer (rfd); these take
//! plain paths so they stay headless.

use std::path::PathBuf;

use bn_core::io;

use crate::doc::{Dirt, Document};
use crate::error::CmdError;
use crate::session::Session;
use crate::views::SaveResult;

pub fn doc_new(s: &mut Session) {
    s.doc = Document::new();
    s.bridge.invalidate();
}

pub fn doc_open(s: &mut Session, path: PathBuf) -> Result<(), CmdError> {
    let iodoc = io::load(&path)?;
    s.doc = Document::from_io_document(iodoc, Some(path));
    s.bridge.invalidate();
    s.finish(Dirt::Structure);
    Ok(())
}

/// Open from already-loaded text (web builds — there is no path to keep).
pub fn doc_open_str(s: &mut Session, text: &str, fmt: io::Format) -> Result<(), CmdError> {
    let iodoc = io::load_str(text, fmt)?;
    s.doc = Document::from_io_document(iodoc, None);
    s.bridge.invalidate();
    s.finish(Dirt::Structure);
    Ok(())
}

/// Serialize for a platform-side save (web builds: the caller downloads the
/// text). Clears `modified` like a successful save; `doc.path` is untouched.
pub fn doc_save_str(s: &mut Session, fmt: io::Format) -> Result<(String, Vec<String>), CmdError> {
    let (text, warnings) = io::save_str(&s.doc.to_io_document(), fmt)?;
    s.doc.modified = false;
    Ok((text, warnings.into_iter().map(|io::Warning::Lossy(m)| m).collect()))
}

/// `path: None` saves to the document's current path (error if it has none —
/// the UI runs a save dialog first in that case).
pub fn doc_save(s: &mut Session, path: Option<PathBuf>) -> Result<SaveResult, CmdError> {
    let path = match path {
        Some(p) => p,
        None => s
            .doc
            .path
            .clone()
            .ok_or_else(|| CmdError::BadRequest("document has no path; use Save As".into()))?,
    };
    let warnings = io::save(&s.doc.to_io_document(), &path)?;
    s.doc.path = Some(path.clone());
    s.doc.modified = false;
    Ok(SaveResult {
        path,
        warnings: warnings.into_iter().map(|io::Warning::Lossy(m)| m).collect(),
    })
}

/// Initial mount: make sure beliefs exist if possible.
pub fn refresh(s: &mut Session) {
    s.finish(Dirt::None);
}
