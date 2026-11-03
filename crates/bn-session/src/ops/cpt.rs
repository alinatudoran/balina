//! CPT / utility-table ops.

use bn_core::model::{NodeId, Table};

use crate::doc::Dirt;
use crate::error::CmdError;
use crate::session::Session;
use crate::views::{check_node, cpt_view, CptView};

pub fn get_cpt(s: &Session, node: NodeId) -> Result<CptView, CmdError> {
    check_node(&s.doc().net, node)?;
    Ok(cpt_view(s.doc(), node))
}

/// Shape check BEFORE `begin_change`: a rejected table (the node's structure
/// changed under a modeless editor) must not clear the redo stack or leave
/// the document marked modified by a begin/undo round trip.
fn check_table_shape(s: &Session, node: NodeId, len: usize) -> Result<(), CmdError> {
    let expected = s.doc().net.table_len(node);
    if len != expected {
        return Err(CmdError::BadRequest(format!(
            "the table needs {expected} values, got {len} — the node's parents or states \
             changed since the editor was opened"
        )));
    }
    Ok(())
}

pub fn set_cpt(s: &mut Session, node: NodeId, data: Vec<f64>) -> Result<(), CmdError> {
    check_node(&s.doc().net, node)?;
    check_table_shape(s, node, data.len())?;
    s.doc_mut().begin_change();
    if let Err(e) = s.doc_mut().net.set_table(node, Table { data }) {
        s.doc_mut().undo();
        return Err(e.into());
    }
    s.doc_mut().net.normalize_table(node);
    s.finish(Dirt::Params);
    Ok(())
}

/// Accept an AI-suggested CPT: the table plus a weak Dirichlet prior
/// (experience ε = 1 per parent configuration), so counting/EM learning lets
/// real data override the suggestion. A node that already carries learned
/// experience keeps it — an AI suggestion must never destroy learned counts.
/// One undoable step.
pub fn set_cpt_ai(s: &mut Session, node: NodeId, data: Vec<f64>) -> Result<(), CmdError> {
    check_node(&s.doc().net, node)?;
    check_table_shape(s, node, data.len())?;
    s.doc_mut().begin_change();
    let rows = s.doc().net.row_count(node);
    let keep_learned = s.doc().net.node(node).experience.is_some();
    let result = s.doc_mut().net.set_table(node, Table { data }).and_then(|()| {
        if keep_learned {
            Ok(())
        } else {
            s.doc_mut().net.set_experience(node, Some(vec![1.0; rows]))
        }
    });
    if let Err(e) = result {
        s.doc_mut().undo();
        return Err(e.into());
    }
    s.doc_mut().net.normalize_table(node);
    s.finish(Dirt::Params);
    Ok(())
}
