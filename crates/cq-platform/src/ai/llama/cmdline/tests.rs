use super::*;

fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(String::from).collect()
}

fn ports(endpoints: Result<Vec<Endpoint>, String>) -> Vec<(u16, bool)> {
    endpoints
        .unwrap()
        .iter()
        .map(|endpoint| (endpoint.port, endpoint.ipv6))
        .collect()
}

#[test]
fn where_llama_server_listens_is_read_from_every_form_of_its_host_and_port() {
    let flags = server(&args("llama-server -m x.gguf --host 0.0.0.0 --port 9000"));
    assert_eq!(flags.host.as_deref(), Some("0.0.0.0"));
    assert_eq!(flags.port.as_deref(), Some("9000"));
    let flags = server(&args("llama-server --host=::1 --port=9001 -m x.gguf"));
    assert_eq!(flags.host.as_deref(), Some("::1"));
    assert_eq!(flags.port.as_deref(), Some("9001"));
    // The last of a repeated flag is the one that counts, and the program's
    // own name is not an argument.
    let flags = server(&args("llama-server --port 1 --port 2"));
    assert_eq!(flags.port.as_deref(), Some("2"));
    let flags = server(&args("--port 5"));
    assert_eq!(flags, ServerFlags::default());
    // A flag with nothing after it gives no value.
    assert_eq!(server(&args("llama-server --port")).port, None);
}

#[test]
fn a_model_flag_in_any_form_means_a_single_model_and_none_means_a_router() {
    for line in [
        "llama-server -m model.gguf",
        "llama-server --model model.gguf",
        "llama-server --model=model.gguf",
        "llama-server -mu https://x/y.gguf",
        "llama-server --model-url https://x/y.gguf",
        "llama-server -hf ggml-org/gemma:Q4",
        "llama-server -hfr a/b",
        "llama-server --hf-repo a/b",
        "llama-server -dr gemma3",
        "llama-server --docker-repo gemma3",
    ] {
        assert!(server(&args(line)).has_model, "{line}");
    }
    for line in [
        "llama-server",
        "llama-server --models-dir ./m --port 8080",
        "llama-server --models-preset p.ini -c 4096",
        // A flag that merely starts with one of them is another flag.
        "llama-server -mm proj.gguf --model-draft d.gguf",
    ] {
        assert!(!server(&args(line)).has_model, "{line}");
    }
}

#[test]
fn a_credential_on_the_command_line_is_told_by_flag_or_by_name() {
    for line in [
        "llama-server -m x --api-key abc",
        "llama-server -m x --api-key=abc",
        "llama-server -m x -hft hf_abc",
        "llama-server -m x --hf-token hf_abc",
        "llama-server -m x --some-Token v",
        "llama-server -m x --client-secret=v",
        "llama-server -m x --admin-password v",
    ] {
        assert!(server(&args(line)).has_secret, "{line}");
    }
    for line in [
        "llama-server -m x --port 8080 --threads 8 --keep 10",
        "llama-server -m x --cache-type-k q8_0 -ngl 99",
        // A value that only looks like one is not a flag.
        "llama-server -m x --alias my-token-model",
    ] {
        assert!(!server(&args(line)).has_secret, "{line}");
    }
}

#[test]
fn certificates_mean_https() {
    assert!(
        server(&args(
            "llama-server -m x --ssl-key-file k --ssl-cert-file c"
        ))
        .tls
    );
    assert!(
        swap(&args(
            "llama-swap -config c.yaml -tls-cert-file c -tls-key-file k"
        ))
        .tls
    );
    assert!(!server(&args("llama-server -m x")).tls);
}

#[test]
fn llama_swaps_listen_address_is_read_with_one_dash_or_two() {
    assert_eq!(
        swap(&args("llama-swap -listen localhost:9090"))
            .listen
            .as_deref(),
        Some("localhost:9090")
    );
    assert_eq!(
        swap(&args("llama-swap --listen=:9091 --config c.yaml"))
            .listen
            .as_deref(),
        Some(":9091")
    );
    assert_eq!(
        swap(&args("llama-swap -listen=[::1]:9092"))
            .listen
            .as_deref(),
        Some("[::1]:9092")
    );
    assert_eq!(swap(&args("llama-swap -config c.yaml")).listen, None);
}

#[test]
fn only_this_machines_own_loopback_is_ever_reached() {
    // llama-server's own default host is 127.0.0.1, port 8080.
    assert_eq!(ports(reach(None, None, SERVER_PORT)), [(8080, false)]);
    assert_eq!(
        ports(reach(None, Some("9000"), SERVER_PORT)),
        [(9000, false)]
    );
    for (host, expected) in [
        ("127.0.0.1", vec![(7, false)]),
        ("0.0.0.0", vec![(7, false)]),
        ("::1", vec![(7, true)]),
        ("[::1]", vec![(7, true)]),
        ("localhost", vec![(7, false), (7, true)]),
        ("::", vec![(7, false), (7, true)]),
        ("[::]", vec![(7, false), (7, true)]),
        ("", vec![(7, false), (7, true)]),
        // A list: one loopback among others is enough, and none repeats.
        ("192.168.1.5,127.0.0.1", vec![(7, false)]),
        ("/run/llama.sock, ::1", vec![(7, true)]),
        ("127.0.0.1,0.0.0.0", vec![(7, false)]),
    ] {
        assert_eq!(
            ports(reach(Some(host), Some("7"), SERVER_PORT)),
            expected,
            "{host:?}"
        );
    }
}

#[test]
fn a_socket_or_a_farther_address_is_left_alone_with_the_reason() {
    let error = reach(Some("/run/llama.sock"), None, SERVER_PORT).unwrap_err();
    assert!(error.contains("UNIX socket"), "{error}");
    for host in [
        "192.168.1.5",
        "llama.lan",
        "10.0.0.1,172.16.0.2",
        "127.0.0.2",
    ] {
        let error = reach(Some(host), None, SERVER_PORT).unwrap_err();
        assert!(
            error.contains(host) && error.contains("loopback"),
            "{error}"
        );
    }
    for port in ["0", "99999", "abc", "-1", ""] {
        let error = reach(None, Some(port), SERVER_PORT).unwrap_err();
        assert!(error.contains("port"), "{port:?}: {error}");
    }
}

#[test]
fn llama_swaps_listen_value_is_host_and_port_with_the_host_left_empty_for_all() {
    assert_eq!(ports(swap_reach(None)), [(8080, false), (8080, true)]);
    assert_eq!(
        ports(swap_reach(Some(":9090"))),
        [(9090, false), (9090, true)]
    );
    assert_eq!(ports(swap_reach(Some("127.0.0.1:9091"))), [(9091, false)]);
    assert_eq!(ports(swap_reach(Some("[::1]:9092"))), [(9092, true)]);
    assert_eq!(
        ports(swap_reach(Some("localhost:9093"))),
        [(9093, false), (9093, true)]
    );
    assert!(
        swap_reach(Some("9094"))
            .unwrap_err()
            .contains("listen address")
    );
    assert!(
        swap_reach(Some("10.0.0.3:9095"))
            .unwrap_err()
            .contains("loopback")
    );
    assert!(
        swap_reach(Some("localhost:x"))
            .unwrap_err()
            .contains("port")
    );
}
