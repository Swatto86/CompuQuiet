//! The replies of llama.cpp and llama-swap, as their own documentation and
//! source give them.

use cq_core::{Endpoint, LoadedModel, ModelServer};

use super::server::{Props, parse_models, parse_props, parse_slots};
use super::swap::{encoded, parse_running};

const AT: Endpoint = Endpoint {
    port: 8080,
    ipv6: false,
};

/// `GET /models` of a router (tools/server/README.md, "Using multiple
/// models"): every model it knows, each with the state of its instance.
const ROUTER_MODELS: &str = r#"{
  "object": "list",
  "data": [
    {
      "id": "ggml-org/gemma-3-4b-it-GGUF:Q4_K_M",
      "aliases": [], "tags": [], "object": "model", "owned_by": "llamacpp",
      "path": "/home/me/.cache/llama.cpp/ggml-org_gemma-3-4b-it-GGUF_gemma-3-4b-it-Q4_K_M.gguf",
      "status": { "value": "loaded", "args": ["llama-server", "-ctx", "4096"] },
      "meta": { "n_params": 3880000000, "size": 2489894912 },
      "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] }
    },
    { "id": "qwen3-8b", "status": { "value": "loaded", "args": [] } },
    { "id": "asleep", "status": { "value": "sleeping", "args": ["llama-server"] } },
    { "id": "loading", "status": { "value": "loading", "args": ["llama-server"] } },
    { "id": "idle", "status": { "value": "unloaded" } },
    { "id": "broken", "status": { "value": "unloaded", "failed": true, "exit_code": 1 } },
    { "id": "fetching", "status": { "value": "downloading", "progress": {} } },
    { "id": "--all", "status": { "value": "loaded" } },
    { "status": { "value": "loaded" } },
    { "id": "no-status" }
  ]
}"#;

fn model(name: &str, bytes: u64, server: ModelServer) -> LoadedModel {
    LoadedModel {
        server,
        name: name.to_string(),
        bytes,
        endpoint: Some(AT),
    }
}

#[test]
fn a_routers_loaded_models_are_listed_with_the_size_their_instance_reports() {
    assert_eq!(
        parse_models(ROUTER_MODELS, AT).unwrap(),
        vec![
            model(
                "ggml-org/gemma-3-4b-it-GGUF:Q4_K_M",
                2_489_894_912,
                ModelServer::LlamaCpp
            ),
            model("qwen3-8b", 0, ModelServer::LlamaCpp),
        ]
    );
    assert!(parse_models(r#"{"data":[]}"#, AT).unwrap().is_empty());
    for bad in ["", "[]", "{}", r#"{"data":{}}"#, "not json"] {
        assert!(parse_models(bad, AT).is_err(), "{bad:?}");
    }
}

#[test]
fn props_tell_a_router_from_a_server_with_a_model_and_say_whether_it_sleeps() {
    // tools/server/server-models.cpp, `get_router_props`.
    let router = r#"{"role":"router","max_instances":4,"models_autoload":true,
        "model_alias":"llama-server","model_path":"none",
        "default_generation_settings":{"params":{},"n_ctx":0}}"#;
    assert_eq!(
        parse_props(router).unwrap(),
        Props {
            router: true,
            sleeping: false,
            slots: true
        }
    );
    // tools/server/README.md, `GET /props` of a server with one model.
    let single = r#"{"total_slots":1,"model_path":"../models/Llama-3.1-8B-Q4_K_M.gguf",
        "chat_template":"...","build_info":"b1-abc","is_sleeping":false}"#;
    assert_eq!(
        parse_props(single).unwrap(),
        Props {
            router: false,
            sleeping: false,
            slots: true
        }
    );
    let asleep = r#"{"total_slots":1,"is_sleeping":true,"endpoint_slots":false}"#;
    assert_eq!(
        parse_props(asleep).unwrap(),
        Props {
            router: false,
            sleeping: true,
            slots: false
        }
    );
    for bad in ["", "[]", "7", "not json"] {
        assert!(parse_props(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn slots_say_whether_any_is_working_and_an_unreadable_list_says_nothing() {
    // tools/server/README.md, `GET /slots`: one object per slot.
    let idle = r#"[{"id":0,"id_task":-1,"n_ctx":4096,"is_processing":false},
                   {"id":1,"id_task":-1,"n_ctx":4096,"is_processing":false}]"#;
    let busy = r#"[{"id":0,"is_processing":false},{"id":1,"id_task":135,"is_processing":true}]"#;
    assert_eq!(parse_slots(idle), Some(false));
    assert_eq!(parse_slots(busy), Some(true));
    assert_eq!(parse_slots("[]"), Some(false));
    for unreadable in ["", "{}", r#"{"error":{"code":501}}"#, "not json"] {
        assert_eq!(parse_slots(unreadable), None, "{unreadable:?}");
    }
}

#[test]
fn llama_swaps_running_models_are_read_from_its_own_shape() {
    // internal/server/api.go, `handleRunning`: {"running": [...]} with each
    // model's id, state and the config it was started from.
    let body = r#"{"running":[
      {"model":"qwen-coder","state":"ready","cmd":"llama-server --port ${PORT} -m q.gguf",
       "proxy":"http://127.0.0.1:5800","ttl":300,"name":"Qwen Coder","description":""},
      {"model":"org/vision 7b","state":"starting","cmd":"","proxy":"","ttl":0,"name":"","description":""},
      {"model":"going","state":"stopping","cmd":"","proxy":"","ttl":0,"name":"","description":""},
      {"model":"","state":"ready"},
      {"model":"a/../b","state":"ready"},
      {"model":"a//b","state":"ready"},
      {"model":"-x","state":"ready"},
      {"state":"ready"}
    ]}"#;
    assert_eq!(
        parse_running(body, AT).unwrap(),
        vec![
            model("qwen-coder", 0, ModelServer::LlamaSwap),
            model("org/vision 7b", 0, ModelServer::LlamaSwap),
        ]
    );
    assert!(parse_running(r#"{"running":[]}"#, AT).unwrap().is_empty());
    for bad in ["", "[]", "{}", r#"{"running":{}}"#, "not json"] {
        assert!(parse_running(bad, AT).is_err(), "{bad:?}");
    }
}

#[test]
fn an_id_becomes_an_address_with_its_slashes_and_nothing_else_unescaped() {
    assert_eq!(encoded("qwen-coder_7b.v2~x"), "qwen-coder_7b.v2~x");
    assert_eq!(encoded("org/model name"), "org/model%20name");
    assert_eq!(encoded("a?b#c%d"), "a%3Fb%23c%25d");
    assert_eq!(encoded("モデル"), "%E3%83%A2%E3%83%87%E3%83%AB");
}
