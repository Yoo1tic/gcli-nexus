pub mod capabilities;
pub mod registry;

pub use capabilities::ModelCapabilities;
pub use registry::ModelRegistry;

use crate::config::{CONFIG, Config};
use std::collections::HashSet;
use std::sync::LazyLock;

pub static MODEL_REGISTRY: LazyLock<ModelRegistry> = LazyLock::new(|| {
    let cfg = &*CONFIG;
    let models = collect_global_model_names(cfg);
    ModelRegistry::new(&models)
});

pub static MODEL_MASK_ALL: LazyLock<u64> = LazyLock::new(|| {
    let model_count = MODEL_REGISTRY.len();
    if model_count >= 64 {
        u64::MAX
    } else {
        (1u64 << model_count) - 1
    }
});

pub fn mask(name: &str) -> Option<u64> {
    MODEL_REGISTRY.get_index(name).map(|idx| 1u64 << idx)
}

/// Resolve a bitmask into a list of model names (best-effort).
///
/// Unknown bits (outside the registry) are ignored here; use `format_model_mask` if you want
/// those shown explicitly in logs.
pub fn model_names_from_mask(model_mask: u64) -> Vec<String> {
    let mut names = Vec::new();
    for idx in 0..MODEL_REGISTRY.len() {
        let bit = 1u64 << idx;
        if (model_mask & bit) != 0 {
            names.push(MODEL_REGISTRY.get_name(idx).to_string());
        }
    }
    names
}

/// Human-friendly formatting for model masks, intended for logs.
pub fn format_model_mask(model_mask: u64) -> String {
    if model_mask == 0 {
        return "[]".to_string();
    }

    let names = model_names_from_mask(model_mask);
    let unknown_bits = model_mask & !*MODEL_MASK_ALL;

    if unknown_bits != 0 {
        format!(
            "[{}] (unknown_bits=0x{:016x})",
            names.join(", "),
            unknown_bits
        )
    } else {
        format!("[{}]", names.join(", "))
    }
}

fn collect_global_model_names(cfg: &Config) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();

    // Provider: geminicli
    let geminicli = cfg.geminicli();
    for name in geminicli.model_list {
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }

    // Provider: codex
    let codex = cfg.codex();
    for name in codex.model_list {
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }

    // Provider: antigravity
    let antigravity = cfg.antigravity();
    for name in antigravity.model_list {
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }

    out
}
