//! CPT / utility-table ops.

use bn_core::model::{NodeId, Table};

use crate::doc::Dirt;
use crate::error::CmdError;
use crate::session::Session;
use crate::views::{check_node, cpt_view, CptView};

pub fn get_cpt(s: &Session, node: NodeId) -> Result<CptView, CmdError> {
    check_node(&s.doc.net, node)?;
    Ok(cpt_view(&s.doc, node))
}

pub fn set_cpt(s: &mut Session, node: NodeId, data: Vec<f64>) -> Result<(), CmdError> {
    check_node(&s.doc.net, node)?;
    s.doc.begin_change();
    if let Err(e) = s.doc.net.set_table(node, Table { data }) {
        s.doc.undo();
        return Err(e.into());
    }
    s.doc.net.normalize_table(node);
    s.finish(Dirt::Params);
    Ok(())
}
