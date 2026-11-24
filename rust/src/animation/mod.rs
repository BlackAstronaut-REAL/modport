pub mod eval;
pub mod legacy;
pub mod preview;
pub mod source;

pub use eval::{evaluate, trig_coefficients};
pub use legacy::{extract_legacy_animation, LegacyAnimation};
pub use preview::bake_blockbench_animations;
pub use source::{extract_java_animation, SourceAnimation};
