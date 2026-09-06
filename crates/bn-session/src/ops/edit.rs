//! Structural + visual edit ops and undo/compile.

use bn_core::io::DisplayMode;
use bn_core::model::{NodeId, NodeKind, State};

use crate::doc::{Dirt, Point};
use crate::error::CmdError;
use crate::session::Session;
use crate::views::{check_node, NodePropsPatch};

// ---- structure (Dirt::Structure) -----------------------------------------

pub fn add_node(s: &mut Session, kind: NodeKind, x: f32, y: f32) -> Result<NodeId, CmdError> {
    let id = s.doc.add_node_at(kind, Point::new(x, y))?;
    s.finish(Dirt::Structure);
    Ok(id)
}

pub fn add_edge(s: &mut Session, parent: NodeId, child: NodeId) -> Result<(), CmdError> {
    check_node(&s.doc.net, parent)?;
    check_node(&s.doc.net, child)?;
    s.doc.begin_change();
    if let Err(e) = s.doc.net.add_edge(parent, child) {
        s.doc.undo();
        return Err(e.into());
    }
    s.finish(Dirt::Structure);
    Ok(())
}

pub fn remove_edge(s: &mut Session, parent: NodeId, child: NodeId) -> Result<(), CmdError> {
    check_node(&s.doc.net, parent)?;
    check_node(&s.doc.net, child)?;
    s.doc.begin_change();
    if let Err(e) = s.doc.net.remove_edge(parent, child) {
        s.doc.undo();
        return Err(e.into());
    }
    s.finish(Dirt::Structure);
    Ok(())
}

/// Detach-and-rewire as ONE undoable step.
pub fn move_edge(
    s: &mut Session,
    parent: NodeId,
    from_child: NodeId,
    to_child: NodeId,
) -> Result<(), CmdError> {
    check_node(&s.doc.net, parent)?;
    check_node(&s.doc.net, from_child)?;
    check_node(&s.doc.net, to_child)?;
    s.doc.begin_change();
    let r = s
        .doc
        .net
        .remove_edge(parent, from_child)
        .and_then(|()| s.doc.net.add_edge(parent, to_child));
    if let Err(e) = r {
        s.doc.undo();
        return Err(e.into());
    }
    s.finish(Dirt::Structure);
    Ok(())
}

pub fn delete_items(
    s: &mut Session,
    nodes: &[NodeId],
    edges: &[(NodeId, NodeId)],
) -> Result<(), CmdError> {
    for &id in nodes {
        check_node(&s.doc.net, id)?;
    }
    for &(a, b) in edges {
        check_node(&s.doc.net, a)?;
        check_node(&s.doc.net, b)?;
    }
    let dirt = s.doc.delete_items(nodes, edges);
    s.finish(dirt);
    Ok(())
}

pub fn update_node_props(
    s: &mut Session,
    node: NodeId,
    patch: NodePropsPatch,
) -> Result<(), CmdError> {
    check_node(&s.doc.net, node)?;
    // Draft validation before touching the document.
    if patch.kind != NodeKind::Utility {
        if patch.states.is_empty() {
            return Err(CmdError::BadRequest("a node needs at least one state".into()));
        }
        let mut names: Vec<&str> = patch.states.iter().map(|st| st.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != patch.states.len() {
            return Err(CmdError::BadRequest("duplicate state names".into()));
        }
    }
    s.doc.begin_change();
    let apply = |s: &mut Session| -> Result<(), CmdError> {
        s.doc.net.rename_node(node, &patch.name)?;
        s.doc.net.set_title(node, patch.title.clone());
        s.doc.net.set_comment(node, patch.comment.clone());
        if s.doc.net.node(node).kind != patch.kind {
            s.doc.net.set_kind(node, patch.kind)?;
        }
        if patch.kind != NodeKind::Utility {
            let new_states: Vec<State> = patch
                .states
                .iter()
                .map(|st| State { name: st.name.clone(), value: st.value })
                .collect();
            let map: Vec<Option<usize>> = patch.states.iter().map(|st| st.orig).collect();
            let identical = map.len() == s.doc.net.node(node).n_states()
                && map.iter().enumerate().all(|(i, o)| *o == Some(i))
                && new_states
                    .iter()
                    .zip(&s.doc.net.node(node).states)
                    .all(|(a, b)| a == b);
            if !identical {
                s.doc.net.remap_states(node, new_states, &map)?;
            }
        }
        Ok(())
    };
    if let Err(e) = apply(s) {
        // Roll the snapshot back so a failed apply leaves no half-edit.
        s.doc.undo();
        return Err(e);
    }
    s.finish(Dirt::Structure);
    Ok(())
}

pub fn set_network_name(s: &mut Session, name: String) {
    s.doc.net.name = name;
    s.doc.modified = true;
}

// ---- visual (undoable, but no model-generation bump) ----------------------

pub fn move_nodes(s: &mut Session, moves: &[(NodeId, f32, f32)]) -> Result<(), CmdError> {
    for &(id, _, _) in moves {
        check_node(&s.doc.net, id)?;
    }
    if moves.is_empty() {
        return Ok(());
    }
    s.doc.begin_visual_change();
    for &(id, x, y) in moves {
        if let Some(v) = s.doc.visual.get_mut(id) {
            v.pos = Point::new(x, y);
        }
    }
    Ok(())
}

pub fn set_display_mode(s: &mut Session, node: NodeId, mode: DisplayMode) -> Result<(), CmdError> {
    check_node(&s.doc.net, node)?;
    s.doc.begin_visual_change();
    if let Some(v) = s.doc.visual.get_mut(node) {
        v.display = mode;
    }
    Ok(())
}

pub fn set_node_color(
    s: &mut Session,
    node: NodeId,
    color: Option<[u8; 3]>,
) -> Result<(), CmdError> {
    check_node(&s.doc.net, node)?;
    s.doc.begin_visual_change();
    if let Some(v) = s.doc.visual.get_mut(node) {
        v.color = color;
    }
    Ok(())
}

// ---- undo / compile --------------------------------------------------------

pub fn undo(s: &mut Session) {
    let dirt = s.doc.undo();
    s.finish(dirt);
}

pub fn redo(s: &mut Session) {
    let dirt = s.doc.redo();
    s.finish(dirt);
}

/// Force a recompute regardless of auto-update (F5 / ⚡ toolbar button).
pub fn recompute(s: &mut Session) {
    if !s.doc.net.is_empty() {
        s.bridge.recompute(&s.doc);
    }
}

pub fn set_auto_update(s: &mut Session, on: bool) {
    s.doc.auto_update = on;
    s.finish(Dirt::None);
}
