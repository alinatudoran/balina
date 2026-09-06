//! Sampling: forward (ancestral) sampling, case generation, and likelihood
//! weighting. All functions take `&mut impl Rng` so tests can seed.

pub mod forward;
pub mod likelihood_weighting;

pub use forward::{forward_sample, generate_cases};
pub use likelihood_weighting::{lw_beliefs, WeightedSample};
