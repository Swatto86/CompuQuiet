//! What a program started again may be given to add to its environment.

use cq_core::{Env, is_carried};

/// The variables of `env` that may be passed on when a program is started
/// again: only the ones `cq_core::is_carried` names, whatever a journal says,
/// and none whose name or value could not be set (a NUL, or an `=` in a name).
pub(crate) fn passed_on(env: &Env) -> impl Iterator<Item = (&str, &str)> {
    env.iter()
        .filter(|(name, value)| {
            is_carried(name) && !name.contains(['=', '\u{0}']) && !value.contains('\u{0}')
        })
        .map(|(name, value)| (name.as_str(), value.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_carried_names_with_settable_names_and_values_are_passed_on() {
        let env = Env::from(
            [
                ("CUDA_VISIBLE_DEVICES", "1"),
                ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "1"),
                ("LLAMA_ARG_N_GPU_LAYERS", "99"),
                // A journal is a file the user can edit: nothing else gets in.
                ("LD_PRELOAD", "/tmp/evil.so"),
                ("PATH", "/tmp"),
                ("GGML_A=B", "x"),
                ("GGML_NUL", "a\u{0}b"),
            ]
            .map(|(name, value)| (name.to_string(), value.to_string())),
        );
        let passed: Vec<_> = passed_on(&env).collect();
        assert_eq!(
            passed,
            [
                ("CUDA_VISIBLE_DEVICES", "1"),
                ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "1"),
                ("LLAMA_ARG_N_GPU_LAYERS", "99"),
            ]
        );
        assert_eq!(passed_on(&Env::new()).count(), 0);
    }
}
