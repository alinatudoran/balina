//! File formats. Each format has its own DTO layer; the shared unit of
//! exchange is [`Document`]: a network plus visual (layout) metadata.
//! A [`Project`] wraps multiple documents as tabs.

pub mod native;
pub mod xdsl;
pub mod xmlbif;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::IoError;
use crate::model::Network;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum DisplayMode {
    TitleOnly,
    #[default]
    BeliefBars,
    ExpectedValue,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeVisual {
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub display: DisplayMode,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
}

/// A free-floating sticky note on the canvas. Pure annotation: not tied to
/// any node and never part of the network model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteInfo {
    #[serde(default)]
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default = "default_note_color")]
    pub color: [u8; 3],
    #[serde(default = "default_note_font")]
    pub font_size: f32,
    #[serde(default)]
    pub collapsed: bool,
}

fn default_note_color() -> [u8; 3] {
    [255, 244, 165] // sticky yellow
}
fn default_note_font() -> f32 {
    13.0
}

/// Layout metadata, keyed by node name (stable across sessions).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VisualInfo {
    pub nodes: HashMap<String, NodeVisual>,
    #[serde(default)]
    pub notes: Vec<NoteInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    pub network: Network,
    pub visual: VisualInfo,
}

// ---------------------------------------------------------------------------
// Project: multi-tab container
// ---------------------------------------------------------------------------

/// One tab in a project.
#[derive(Clone, Debug)]
pub struct ProjectSheet {
    pub label: String,
    pub doc: Document,
}

/// A multi-tab project.
#[derive(Clone, Debug)]
pub struct Project {
    pub sheets: Vec<ProjectSheet>,
    pub active: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    NativeJson,
    Xmlbif,
    Xdsl,
}

impl Format {
    pub fn from_path(path: &Path) -> Result<Format, IoError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "balina" | "json" => Ok(Format::NativeJson),
            "xml" | "bif" | "xmlbif" => Ok(Format::Xmlbif),
            "xdsl" => Ok(Format::Xdsl),
            other => Err(IoError::UnknownFormat(other.to_string())),
        }
    }
}

/// Non-fatal information loss during save/load.
#[derive(Clone, Debug)]
pub enum Warning {
    Lossy(String),
}

// ---------------------------------------------------------------------------
// Single-document load/save (XMLBIF, XDSL)
// ---------------------------------------------------------------------------

pub fn load(path: &Path) -> Result<Document, IoError> {
    let fmt = Format::from_path(path)?;
    let text = std::fs::read_to_string(path)?;
    load_str(&text, fmt)
}

pub fn load_str(text: &str, fmt: Format) -> Result<Document, IoError> {
    match fmt {
        Format::NativeJson => {
            // For single-doc load, take the active tab from the project.
            let proj = native::project_from_json(text)?;
            let idx = proj.active.min(proj.sheets.len().saturating_sub(1));
            Ok(proj.sheets.into_iter().nth(idx).map(|s| s.doc).unwrap_or_default())
        }
        Format::Xmlbif => xmlbif::from_xml(text),
        Format::Xdsl => xdsl::from_xml(text),
    }
}

pub fn save(doc: &Document, path: &Path) -> Result<Vec<Warning>, IoError> {
    let fmt = Format::from_path(path)?;
    let (text, warnings) = save_str(doc, fmt)?;
    std::fs::write(path, text)?;
    Ok(warnings)
}

pub fn save_str(doc: &Document, fmt: Format) -> Result<(String, Vec<Warning>), IoError> {
    match fmt {
        Format::NativeJson => {
            let project = Project {
                sheets: vec![ProjectSheet { label: doc.network.name.clone(), doc: doc.clone() }],
                active: 0,
            };
            Ok((native::project_to_json(&project)?, vec![]))
        }
        Format::Xmlbif => xmlbif::to_xml(doc),
        Format::Xdsl => xdsl::to_xml(doc),
    }
}

// ---------------------------------------------------------------------------
// Project load/save (native JSON only)
// ---------------------------------------------------------------------------

pub fn load_project(path: &Path) -> Result<Project, IoError> {
    let text = std::fs::read_to_string(path)?;
    load_project_str(&text)
}

pub fn load_project_str(text: &str) -> Result<Project, IoError> {
    native::project_from_json(text)
}

pub fn save_project(project: &Project, path: &Path) -> Result<(), IoError> {
    let text = save_project_str(project)?;
    std::fs::write(path, text)?;
    Ok(())
}

pub fn save_project_str(project: &Project) -> Result<String, IoError> {
    native::project_to_json(project)
}
