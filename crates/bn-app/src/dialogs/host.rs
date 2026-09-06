//! Renders whichever dialog is open, and closes node-bound dialogs when
//! their node vanishes from the document.

use dioxus::prelude::*;

use crate::state::{close_dialog, DialogDesc, DIALOG, SESSION};

#[component]
pub fn DialogHost() -> Element {
    // Close-on-vanish: subscribe to both the dialog slot and the session.
    use_effect(move || {
        let node = match &*DIALOG.read() {
            Some(DialogDesc::NodeProperties { node })
            | Some(DialogDesc::CptEditor { node })
            | Some(DialogDesc::Likelihood { node }) => Some(*node),
            _ => None,
        };
        if let Some(id) = node
            && !SESSION.read().doc.net.contains(id) {
                close_dialog();
            }
    });

    let Some(dialog) = DIALOG.read().clone() else { return rsx! {} };
    match dialog {
        DialogDesc::NodeProperties { node } => rsx! {
            crate::dialogs::node_props::NodePropertiesDialog { key: "{node:?}", id: node }
        },
        DialogDesc::CptEditor { node } => rsx! {
            crate::dialogs::cpt_editor::CptEditorDialog { key: "{node:?}", id: node }
        },
        DialogDesc::Likelihood { node } => rsx! {
            crate::dialogs::misc::LikelihoodDialog { key: "{node:?}", id: node }
        },
        DialogDesc::LearnCpts => rsx! {
            crate::dialogs::learn_cpts::LearnCptsDialog {}
        },
        DialogDesc::StructureLearn => rsx! {
            crate::dialogs::structure_learn::StructureLearnDialog {}
        },
        DialogDesc::Simulate => rsx! {
            crate::dialogs::simulate::SimulateDialog {}
        },
        DialogDesc::Sensitivity => rsx! {
            crate::dialogs::sensitivity::SensitivityDialog {}
        },
        DialogDesc::ArcStrength => rsx! {
            crate::dialogs::arc_strength::ArcStrengthDialog {}
        },
        DialogDesc::IdSolution { text } => rsx! {
            crate::dialogs::misc::ReportDialog {
                title: "Influence diagram solution".to_string(),
                text,
            }
        },
        DialogDesc::RenameNetwork => rsx! {
            crate::dialogs::misc::RenameNetworkDialog {}
        },
        DialogDesc::About => rsx! {
            crate::dialogs::misc::AboutDialog {}
        },
    }
}
