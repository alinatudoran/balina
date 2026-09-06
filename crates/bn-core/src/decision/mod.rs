//! Decision networks (influence diagrams).

pub mod single;
pub mod ve_id;

pub use single::expected_utilities;
pub use ve_id::{solve_influence_diagram, IdSolution, Policy};
