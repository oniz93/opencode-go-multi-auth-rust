pub mod catalog;
pub mod merge;
pub mod opencode;

pub use catalog::{fetch_models, lane_for, parse_el_catalog, parse_model_list, Lane, ModelInfo};
pub use merge::Catalog;
