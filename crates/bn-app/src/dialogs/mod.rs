//! Dialogs. One open at a time (`state::DIALOG`); node-bound dialogs close
//! automatically when their node vanishes (undo, delete, learning).

pub mod arc_strength;
pub mod cpt_editor;
pub mod host;
pub mod learn_cpts;
pub mod misc;
pub mod node_props;
pub mod sensitivity;
pub mod simulate;
pub mod structure_learn;
