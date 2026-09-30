use super::*;

fn model(server: ModelServer, name: &str) -> LoadedModel {
    LoadedModel {
        server,
        name: name.to_string(),
        bytes: 4_000_000_000,
        endpoint: None,
    }
}

fn server_close(pid: u32) -> ServerClose {
    ServerClose {
        pid,
        name: "llama-server".into(),
        start_time: 9,
        exe: PathBuf::from("C:/llama/llama-server.exe"),
        args: vec!["llama-server.exe".into(), "-m".into(), "qwen.gguf".into()],
        cwd: Some(PathBuf::from("C:/llama")),
        env: Env::from([("CUDA_VISIBLE_DEVICES".to_string(), "1".to_string())]),
    }
}

#[test]
fn each_model_becomes_a_step_before_the_purge_and_after_the_rest() {
    let mut plan = Plan {
        steps: vec![Step::SetPerformancePower, Step::PurgeMemory],
        skipped: Vec::new(),
    };
    plan_unloads(
        &mut plan,
        ModelServers {
            loaded: vec![
                model(ModelServer::Ollama, "llama3:8b"),
                model(ModelServer::LmStudio, "qwen/qwen3-4b"),
            ],
            ..ModelServers::default()
        },
    );
    let unloaded: Vec<&str> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::UnloadModel { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(unloaded, ["llama3:8b", "qwen/qwen3-4b"]);
    assert_eq!(plan.steps[0], Step::SetPerformancePower);
    assert_eq!(plan.steps[3], Step::PurgeMemory, "{:?}", plan.steps);
    assert!(plan.skipped.is_empty());
}

#[test]
fn models_are_added_at_the_end_when_there_is_no_purge() {
    let mut plan = Plan {
        steps: vec![Step::KeepAwake],
        skipped: Vec::new(),
    };
    plan_unloads(
        &mut plan,
        ModelServers {
            loaded: vec![model(ModelServer::Ollama, "a")],
            ..ModelServers::default()
        },
    );
    assert!(matches!(plan.steps[1], Step::UnloadModel { .. }));
}

#[test]
fn nothing_loaded_is_said_and_a_server_that_could_not_be_asked_is_said_instead() {
    let mut plan = Plan::default();
    plan_unloads(&mut plan, ModelServers::default());
    assert!(plan.steps.is_empty());
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].name, "AI models");

    let mut plan = Plan::default();
    plan_unloads(
        &mut plan,
        ModelServers {
            skipped: vec![Skipped {
                name: "LM Studio".into(),
                reason: "its tool failed".into(),
            }],
            ..ModelServers::default()
        },
    );
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].name, "LM Studio");
}

#[test]
fn an_unload_has_no_journal_entry_so_older_releases_can_read_the_journal() {
    let step = Step::UnloadModel {
        server: ModelServer::Ollama,
        name: "llama3:8b".into(),
        bytes: 1,
        endpoint: None,
    };
    assert_eq!(crate::DoneStep::intended(&step, None), None);
    assert_eq!(step.label(), "Unload llama3:8b from Ollama");
}

#[test]
fn a_single_model_server_becomes_a_close_after_the_unloads_and_before_the_purge() {
    let mut plan = Plan {
        steps: vec![Step::SetPerformancePower, Step::PurgeMemory],
        skipped: Vec::new(),
    };
    plan_unloads(
        &mut plan,
        ModelServers {
            loaded: vec![model(ModelServer::LlamaCpp, "gemma")],
            closes: vec![server_close(7)],
            ..ModelServers::default()
        },
    );
    assert!(matches!(plan.steps[1], Step::UnloadModel { .. }));
    assert_eq!(plan.steps[2], Step::CloseModelServer(server_close(7)));
    assert_eq!(plan.steps[3], Step::PurgeMemory);
    assert!(plan.skipped.is_empty(), "a close is something found");
    assert_eq!(plan.steps[2].label(), "Close llama-server (PID 7)");
}

#[test]
fn a_close_is_journaled_with_its_variables_so_restore_can_start_it_the_same() {
    let step = Step::CloseModelServer(server_close(7));
    let done = crate::DoneStep::intended(&step, None).expect("a close is journaled");
    let crate::DoneStep::ProcessClosed { exe, args, env, .. } = &done else {
        panic!("{done:?}");
    };
    assert_eq!(
        exe.as_deref(),
        Some(std::path::Path::new("C:/llama/llama-server.exe"))
    );
    assert_eq!(args.len(), 3);
    assert_eq!(env["CUDA_VISIBLE_DEVICES"], "1");
    let Some(crate::RestoreStep::Relaunch { env, .. }) = done.restore() else {
        panic!("a close is undone by starting it again");
    };
    assert_eq!(env["CUDA_VISIBLE_DEVICES"], "1");
}

#[test]
fn only_the_variables_that_decide_how_llama_cpp_runs_are_carried() {
    let environment = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    };
    let kept = carried(&environment(&[
        ("PATH", "/usr/bin"),
        ("HOME", "/home/me"),
        ("CUDA_VISIBLE_DEVICES", "0,1"),
        ("hip_visible_devices", "0"),
        ("ROCR_VISIBLE_DEVICES", "1"),
        ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "1"),
        ("LLAMA_ARG_N_GPU_LAYERS", "99"),
        // What makes a card index mean the same card, and where -hf finds
        // its model.
        ("CUDA_DEVICE_ORDER", "PCI_BUS_ID"),
        ("GPU_DEVICE_ORDINAL", "1"),
        ("HSA_OVERRIDE_GFX_VERSION", "10.3.0"),
        ("ONEAPI_DEVICE_SELECTOR", "level_zero:1"),
        ("LLAMA_CACHE", "/cache"),
        // Not carried, so neither kept nor a reason to refuse.
        ("HF_TOKEN", "hf_secret"),
        ("OPENAI_API_KEY", "sk-x"),
    ]))
    .unwrap();
    assert_eq!(
        kept.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "CUDA_DEVICE_ORDER",
            "CUDA_VISIBLE_DEVICES",
            "GGML_CUDA_ENABLE_UNIFIED_MEMORY",
            "GPU_DEVICE_ORDINAL",
            "HSA_OVERRIDE_GFX_VERSION",
            "LLAMA_ARG_N_GPU_LAYERS",
            "LLAMA_CACHE",
            "ONEAPI_DEVICE_SELECTOR",
            "ROCR_VISIBLE_DEVICES",
            "hip_visible_devices",
        ]
    );
    assert!(carried(&[]).unwrap().is_empty());
}

#[test]
fn a_credential_among_the_carried_variables_refuses_the_lot() {
    for name in [
        "LLAMA_ARG_API_KEY_FILE",
        "GGML_TOKEN",
        "llama_arg_secret",
        "LLAMA_ARG_PASSWORD",
    ] {
        let error = carried(&[
            ("CUDA_VISIBLE_DEVICES".into(), "0".into()),
            (name.into(), "x".into()),
        ])
        .unwrap_err();
        assert_eq!(error, name);
    }
}

#[test]
fn the_endpoint_reads_as_an_address_and_a_step_survives_a_round_trip() {
    assert_eq!(
        Endpoint {
            port: 8080,
            ipv6: false
        }
        .to_string(),
        "127.0.0.1:8080"
    );
    assert_eq!(
        Endpoint {
            port: 9,
            ipv6: true
        }
        .to_string(),
        "[::1]:9"
    );
    for step in [
        Step::CloseModelServer(server_close(3)),
        Step::UnloadModel {
            server: ModelServer::LlamaSwap,
            name: "a/b".into(),
            bytes: 0,
            endpoint: Some(Endpoint {
                port: 8081,
                ipv6: false,
            }),
        },
    ] {
        let json = serde_json::to_string(&step).unwrap();
        assert_eq!(serde_json::from_str::<Step>(&json).unwrap(), step, "{json}");
    }
    // A step saved before the address existed still reads.
    let old: Step =
        serde_json::from_str(r#"{"kind":"unload_model","server":"ollama","name":"x","bytes":1}"#)
            .unwrap();
    assert!(matches!(old, Step::UnloadModel { endpoint: None, .. }));
}
