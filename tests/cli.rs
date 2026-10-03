use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("ma-test-{}-{stamp}-{n}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cli() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ma"));
    for name in [
        "BASE_URL",
        "API_KEY",
        "MODEL",
        "MA_BASE_URL",
        "MA_API_KEY",
        "MA_MODEL",
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_MODEL",
        "MA_CONFIG",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        c.env_remove(name);
    }
    // Keep the default config location empty so a test never reads the developer's real
    // ~/.config/ma/config.json; config tests opt in through MA_CONFIG.
    c.env(
        "XDG_CONFIG_HOME",
        std::env::temp_dir().join(format!("ma-test-no-config-{}", std::process::id())),
    );
    c
}

fn read_request(stream: &mut TcpStream) -> Value {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "POST /v1/chat/completions HTTP/1.1");
    let mut length = None;
    let mut authorized = false;
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (key, value) = line.trim().split_once(':').unwrap();
        if key.eq_ignore_ascii_case("content-length") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
        if key.eq_ignore_ascii_case("authorization") {
            authorized = value.trim() == "Bearer test-key";
        }
    }
    assert!(authorized, "model request must carry API key");
    let mut body = vec![0; length.unwrap()];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn run(
    responses: Vec<(&'static str, Value)>,
    flags: &[&str],
    input: &str,
    workspace: &Workspace,
) -> (Output, Vec<Value>) {
    run_custom(responses, flags, input, workspace, None, |_| {})
}

fn run_with_env(
    responses: Vec<(&'static str, Value)>,
    flags: &[&str],
    input: &str,
    workspace: &Workspace,
    env: Option<(&str, &std::ffi::OsStr)>,
) -> (Output, Vec<Value>) {
    run_custom(responses, flags, input, workspace, env, |_| {})
}

fn run_custom(
    responses: Vec<(&'static str, Value)>,
    flags: &[&str],
    input: &str,
    workspace: &Workspace,
    env: Option<(&str, &std::ffi::OsStr)>,
    after_spawn: impl FnOnce(&mut std::process::Child),
) -> (Output, Vec<Value>) {
    run_wire(
        responses
            .into_iter()
            .map(|(status, body)| (status, Reply::Json(body)))
            .collect(),
        flags,
        input,
        workspace,
        env,
        after_spawn,
    )
}

enum Reply {
    Json(Value),
    Sse(Vec<Vec<u8>>),
}

fn serve(
    listener: TcpListener,
    responses: Vec<(&'static str, Reply)>,
) -> thread::JoinHandle<Vec<Value>> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return requests;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            requests.push(read_request(&mut stream));
            match body {
                Reply::Json(body) => {
                    let body = body.to_string();
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
                Reply::Sse(chunks) => {
                    let sent = (|| -> std::io::Result<()> {
                        write!(
                            stream,
                            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream; charset=utf-8\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                        )?;
                        for chunk in chunks {
                            write!(stream, "{:x}\r\n", chunk.len())?;
                            stream.write_all(&chunk)?;
                            stream.write_all(b"\r\n")?;
                            stream.flush()?;
                        }
                        stream.write_all(b"0\r\n\r\n")
                    })();
                    if let Err(e) = sent {
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                        ) {
                            return requests;
                        }
                        panic!("SSE send: {e}");
                    }
                }
            }
        }
        requests
    })
}

fn run_wire(
    responses: Vec<(&'static str, Reply)>,
    flags: &[&str],
    input: &str,
    workspace: &Workspace,
    env: Option<(&str, &std::ffi::OsStr)>,
    after_spawn: impl FnOnce(&mut std::process::Child),
) -> (Output, Vec<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = serve(listener, responses);
    let mut command = cli();
    if let Some((key, value)) = env {
        command.env(key, value);
    }
    let mut child = command
        .current_dir(&workspace.0)
        .args([
            "--base-url",
            &url,
            "--model",
            "test-model",
            "--http-timeout",
            "3",
        ])
        .env("API_KEY", "test-key")
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    after_spawn(&mut child);
    let output = child.wait_with_output().unwrap();
    (output, server.join().unwrap())
}

fn final_response(text: &str) -> Value {
    json!({"id":"test", "object":"chat.completion", "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":text}}]})
}

fn tools(commands: &[(&str, &str)]) -> Value {
    let calls: Vec<Value> = commands.iter().map(|(id, command)| json!({"id":id,"type":"function","function":{"name":"shell","arguments":json!({"command":command}).to_string()}})).collect();
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"reasoning_content":"plan","tool_calls":calls}}]})
}

fn result(request: &Value, index: usize) -> Value {
    serde_json::from_str(request["messages"][index]["content"].as_str().unwrap()).unwrap()
}

#[test]
fn final_output_and_piped_context_use_only_stdout() {
    let ws = Workspace::new();
    let (out, requests) = run(
        vec![("200 OK", final_response("完成"))],
        &["总结修改"],
        "diff context",
        &ws,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "完成\n");
    assert!(out.stderr.is_empty());
    assert_eq!(requests[0]["model"], "test-model");
    let user = requests[0]["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("总结修改") && user.contains("diff context"));
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(requests[0]["tools"][0]["function"]["name"], "shell");
    assert_eq!(requests[0]["stream"], true);
}

#[test]
fn multiple_tools_and_turns_preserve_assistant_and_results() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("one", "printf hello"),
                    ("two", "printf problem >&2; exit 7"),
                ]),
            ),
            ("200 OK", tools(&[("three", "pwd")])),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(req.len(), 3);
    assert_eq!(req[1]["messages"][2]["reasoning_content"], "plan");
    assert_eq!(req[1]["messages"][3]["tool_call_id"], "one");
    assert_eq!(result(&req[1], 3)["stdout"], "hello");
    assert_eq!(result(&req[1], 4)["stderr"], "problem");
    assert_eq!(result(&req[1], 4)["exit_code"], 7);
    assert_eq!(req[2]["messages"][6]["tool_call_id"], "three");
}

#[test]
fn default_write_denial_is_a_tool_result_and_does_not_write() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            ("200 OK", tools(&[("one", "printf unsafe > denied")])),
            ("200 OK", final_response("permission required")),
        ],
        &["try writing"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert!(!ws.0.join("denied").exists());
    assert!(
        result(&req[1], 3)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
}

#[test]
fn write_allows_workspace_but_denies_parent_and_symlink_escape() {
    use std::os::unix::fs::symlink;
    let ws = Workspace::new();
    let outside = Workspace::new();
    symlink(&outside.0, ws.0.join("escape")).unwrap();
    let outside_command = format!("touch {}", outside.0.join("absolute").display());
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("one", "mkdir sub && printf yes > sub/file"),
                    ("two", "touch escape/escaped"),
                    ("three", &outside_command),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--write", "edit"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(
        std::fs::read_to_string(ws.0.join("sub/file")).unwrap(),
        "yes"
    );
    assert!(!outside.0.join("escaped").exists());
    assert!(!outside.0.join("absolute").exists());
    assert!(
        result(&req[1], 4)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
    assert!(
        result(&req[1], 5)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
}

#[test]
fn shell_network_denial_does_not_block_model_http() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            ("200 OK", tools(&[("net", "curl https://example.com")])),
            ("200 OK", final_response("network required")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert!(
        result(&req[1], 3)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
}

#[test]
fn tool_timeout_is_returned_and_descendants_do_not_hold_pipes_open() {
    let ws = Workspace::new();
    let started = Instant::now();
    let (out, req) = run(
        vec![
            ("200 OK", tools(&[("slow", "sleep 20 & wait")])),
            ("200 OK", final_response("timed out")),
        ],
        &["--shell-timeout", "1", "inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(result(&req[1], 3)["timed_out"], true);
}

#[test]
fn shell_output_is_bounded_and_api_key_is_not_inherited() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("large", "yes x | head -c 100000"),
                    ("secret", "printenv API_KEY"),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(result(&req[1], 3)["stdout"].as_str().unwrap().len(), 65536);
    assert_eq!(result(&req[1], 3)["truncated"], true);
    assert_eq!(result(&req[1], 4)["stdout"], "");
}

#[test]
fn step_limit_does_not_execute_last_unreportable_tool() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![("200 OK", tools(&[("write", "touch never")]))],
        &["--write", "--max-steps", "1", "edit"],
        "",
        &ws,
    );
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(req.len(), 1);
    assert!(!ws.0.join("never").exists());
    assert!(out.stdout.is_empty());
}

#[test]
fn invalid_tool_arguments_are_reported_without_execution() {
    let ws = Workspace::new();
    let response = json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"bad","type":"function","function":{"name":"shell","arguments":"not JSON"}}]}}]});
    let (out, req) = run(
        vec![
            ("200 OK", response),
            ("200 OK", final_response("recovered")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert!(
        result(&req[1], 3)["error"]
            .as_str()
            .unwrap()
            .contains("arguments")
    );
}

#[test]
fn http_failure_and_truncated_completion_fail_without_final_stdout() {
    let ws = Workspace::new();
    let (out, _) = run(
        vec![(
            "401 Unauthorized",
            json!({"error":{"message":"invalid key"}}),
        )],
        &["inspect"],
        "",
        &ws,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("401"));
    let response = json!({"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"partial"}}]});
    let (out, _) = run(vec![("200 OK", response)], &["inspect"], "", &ws);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn cli_rejects_bad_configuration_before_network_and_supports_help() {
    for args in [
        &["--max-steps", "0", "inspect"][..],
        &["--shell-timeout", "no", "inspect"],
        &["--unknown"],
        &["inspect"],
    ] {
        let out = cli().args(args).stdin(Stdio::null()).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
    }
    let out = cli().arg("--help").stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("--write"));
}

#[test]
fn quoted_shell_operators_are_data_and_output_flags_obey_permissions() {
    let ws = Workspace::new();
    std::fs::write(ws.0.join("input"), "b\na\n").unwrap();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("literal", "printf '%s' '>'"),
                    ("sort", "sort input -o denied"),
                    ("git", "git diff --output=denied-diff"),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(result(&req[1], 3)["stdout"], ">");
    assert!(!ws.0.join("denied").exists());
    assert!(
        result(&req[1], 4)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
    assert!(
        result(&req[1], 5)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
}

#[test]
fn malformed_ids_and_unknown_tools_never_execute_shell() {
    let ws = Workspace::new();
    let mut duplicate = tools(&[("same", "touch never"), ("same", "touch never")]);
    let (out, _) = run(
        vec![("200 OK", duplicate.clone())],
        &["--write", "edit"],
        "",
        &ws,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!ws.0.join("never").exists());
    duplicate["choices"][0]["message"]["tool_calls"] = json!([{"id":"unknown","type":"function","function":{"name":"bash","arguments":"{\"command\":\"touch never\"}"}}]);
    let (out, req) = run(
        vec![("200 OK", duplicate), ("200 OK", final_response("done"))],
        &["--write", "edit"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert!(!ws.0.join("never").exists());
    assert!(
        result(&req[1], 3)["error"]
            .as_str()
            .unwrap()
            .contains("unknown tool")
    );
}

#[test]
fn write_checks_parent_paths_and_command_forwarders() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("parent", "touch ../outside"),
                    ("nested", "sh -c 'touch ../outside'"),
                    ("find", "find . -exec touch ../outside \\;"),
                    ("dynamic", "printf x > \"$HOME/outside\""),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--write", "edit"],
        "",
        &ws,
    );
    assert!(out.status.success());
    for index in 3..7 {
        assert!(
            result(&req[1], index)["error"]
                .as_str()
                .unwrap()
                .contains("permission denied")
        );
    }
}

#[test]
fn cd_in_subshells_does_not_authorize_outside_writes() {
    let ws = Workspace::new();
    std::fs::create_dir(ws.0.join("sub")).unwrap();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("pipe", "cd sub | touch ../ma-guard-outside-probe"),
                    ("background", "cd sub & touch ../ma-guard-outside-probe"),
                    ("branch", "cd sub || touch ../ma-guard-outside-probe"),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--write", "inspect"],
        "",
        &ws,
    );
    // Clean up any file produced by the intentionally failing pre-fix regression.
    let outside = ws.0.parent().unwrap().join("ma-guard-outside-probe");
    let escaped = outside.exists();
    let _ = std::fs::remove_file(&outside);
    assert!(out.status.success());
    assert!(
        !escaped,
        "cd in a subshell must not change the guard's parent cwd"
    );
    for index in 3..6 {
        assert!(
            result(&req[1], index)["error"]
                .as_str()
                .unwrap()
                .contains("permission denied")
        );
    }
}

#[test]
fn network_only_does_not_enable_implicit_download_writes() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("download", "wget http://127.0.0.1:1/test"),
                    ("remote_name", "curl -O http://127.0.0.1:1/test"),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--net", "inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    for index in 3..5 {
        assert!(
            result(&req[1], index)["error"]
                .as_str()
                .unwrap()
                .contains("permission denied")
        );
    }
}

#[test]
fn shell_ignores_cdpath_and_denies_new_symlink_escapes() {
    let ws = Workspace::new();
    let outside = Workspace::new();
    std::fs::create_dir(ws.0.join("sub")).unwrap();
    std::fs::create_dir(outside.0.join("sub")).unwrap();
    let (out, req) = run_with_env(
        vec![
            (
                "200 OK",
                tools(&[
                    ("cd", "cd sub && touch marker"),
                    (
                        "link",
                        "cd sub && ln -s .. ../link && touch ../link/ma-symlink-probe",
                    ),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--write", "edit"],
        "",
        &ws,
        Some(("CDPATH", outside.0.as_os_str())),
    );
    let escaped = outside.0.join("sub/marker").exists();
    let probe = ws.0.parent().unwrap().join("ma-symlink-probe");
    let symlink_escaped = probe.exists();
    let _ = std::fs::remove_file(probe);
    assert!(out.status.success());
    assert!(!escaped && !symlink_escaped);
    assert!(ws.0.join("sub/marker").exists());
    assert!(
        result(&req[1], 4)["error"]
            .as_str()
            .unwrap()
            .contains("permission denied")
    );
}

#[test]
fn termination_cleans_up_running_shell_group() {
    let ws = Workspace::new();
    let started = Instant::now();
    let (out, _) = run_custom(
        vec![(
            "200 OK",
            tools(&[("long", "touch started; sleep 2; touch survivor")]),
        )],
        &["--write", "edit"],
        "",
        &ws,
        None,
        |child| {
            while !ws.0.join("started").exists() {
                assert!(started.elapsed() < Duration::from_secs(3));
                thread::sleep(Duration::from_millis(5));
            }
            // SAFETY: send SIGTERM only to the test-owned ma process.
            assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
        },
    );
    thread::sleep(Duration::from_millis(2200));
    assert!(
        !ws.0.join("survivor").exists(),
        "terminated ma left its tool running"
    );
    assert_eq!(out.status.code(), Some(128 + libc::SIGTERM));
}

#[test]
fn stdin_only_is_a_task_and_oversized_input_fails_before_http() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![("200 OK", final_response("done"))],
        &[],
        "inspect from stdin",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(req[0]["messages"][1]["content"], "inspect from stdin");
    let (out, req) = run(vec![], &["inspect"], &"x".repeat(1024 * 1024 + 1), &ws);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(req.is_empty());
}

#[test]
fn http_timeout_ends_a_stalled_model_request_without_stdout() {
    let ws = Workspace::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        thread::sleep(Duration::from_secs(2));
    });
    let started = Instant::now();
    let out = cli()
        .current_dir(&ws.0)
        .env("API_KEY", "test-key")
        .args([
            "--base-url",
            &url,
            "--model",
            "test-model",
            "--http-timeout",
            "1",
            "inspect",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        elapsed < Duration::from_millis(1800),
        "request took {elapsed:?}"
    );
}

fn chunk(delta: Value, finish: Value) -> Value {
    json!({"object":"chat.completion.chunk","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}

fn sse(events: &[Value], done: bool) -> Reply {
    let mut wire = String::from(": heartbeat\r\n\r\n");
    for event in events {
        wire.push_str(&format!("data: {event}\r\n\r\n"));
    }
    if done {
        wire.push_str("data: [DONE]\r\n\r\n");
    }
    // Transport fragments split CRLF, JSON escapes and Chinese UTF-8 codepoints.
    Reply::Sse(wire.as_bytes().chunks(3).map(<[u8]>::to_vec).collect())
}

#[test]
fn streaming_text_is_reassembled_and_quiet_mode_keeps_stderr_empty() {
    let ws = Workspace::new();
    let (out, req) = run_wire(
        vec![(
            "200 OK",
            sse(
                &[
                    chunk(json!({"role":"assistant","content":""}), Value::Null),
                    chunk(json!({"content":"你"}), Value::Null),
                    chunk(json!({"content":"好\nworld"}), Value::Null),
                    chunk(json!({}), json!("stop")),
                    json!({"choices":[],"usage":{"total_tokens":10}}),
                ],
                true,
            ),
        )],
        &["inspect"],
        "",
        &ws,
        None,
        |_| {},
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, "你好\nworld\n".as_bytes());
    assert!(out.stderr.is_empty());
    assert_eq!(req[0]["stream"], true);
}

#[test]
fn streaming_tool_arguments_and_reasoning_survive_interleaved_calls() {
    let ws = Workspace::new();
    let response = sse(
        &[
            chunk(
                json!({"role":"assistant","reasoning_content":"first ","tool_calls":[{"index":1,"id":"two","type":"function","function":{"name":"shell","arguments":"{\"command\":\"printf "}},{"index":0,"id":"one","type":"function","function":{"name":"shell","arguments":"{\"command\":\"printf "}}]}),
                Value::Null,
            ),
            chunk(
                json!({"reasoning_content":"second","tool_calls":[{"index":0,"function":{"arguments":"hello\"}"}},{"index":1,"function":{"arguments":"problem >&2; exit 7\"}"}}]}),
                Value::Null,
            ),
            chunk(json!({}), json!("tool_calls")),
        ],
        true,
    );
    let (out, req) = run_wire(
        vec![
            ("200 OK", response),
            (
                "200 OK",
                sse(
                    &[
                        chunk(json!({"content":"done"}), Value::Null),
                        chunk(json!({}), json!("stop")),
                    ],
                    true,
                ),
            ),
        ],
        &["inspect"],
        "",
        &ws,
        None,
        |_| {},
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(req[1]["messages"][2]["reasoning_content"], "first second");
    assert_eq!(req[1]["messages"][3]["tool_call_id"], "one");
    assert_eq!(result(&req[1], 3)["stdout"], "hello");
    assert_eq!(req[1]["messages"][4]["tool_call_id"], "two");
    assert_eq!(result(&req[1], 4)["exit_code"], 7);
    assert_eq!(out.stdout, b"done\n");
}

#[test]
fn streaming_accepts_ollama_style_reasoning_field() {
    let ws = Workspace::new();
    // Ollama-compatible gateways stream reasoning under `reasoning`, not `reasoning_content`.
    let (out, req) = run_wire(
        vec![
            (
                "200 OK",
                sse(
                    &[
                        chunk(
                            json!({"role":"assistant","reasoning":"we need ","tool_calls":[{"index":0,"id":"one","type":"function","function":{"name":"shell","arguments":"{\"command\":\"printf hi\"}"}}]}),
                            Value::Null,
                        ),
                        chunk(json!({"reasoning":"to check"}), json!("tool_calls")),
                    ],
                    true,
                ),
            ),
            (
                "200 OK",
                sse(
                    &[
                        chunk(json!({"content":"done"}), Value::Null),
                        chunk(json!({}), json!("stop")),
                    ],
                    true,
                ),
            ),
        ],
        &["inspect"],
        "",
        &ws,
        None,
        |_| {},
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(req[1]["messages"][2]["reasoning"], "we need to check");
    assert_eq!(result(&req[1], 3)["stdout"], "hi");
    assert_eq!(out.stdout, b"done\n");
}

#[test]
fn incomplete_or_malformed_streams_never_print_final_or_execute_tools() {
    let ws = Workspace::new();
    let call = chunk(
        json!({"tool_calls":[{"index":0,"id":"write","type":"function","function":{"name":"shell","arguments":"{\"command\":\"touch never\"}"}}]}),
        json!("tool_calls"),
    );
    for response in [
        sse(&[call], false),
        Reply::Sse(vec![b"data: broken-json\n\ndata: [DONE]\n\n".to_vec()]),
        sse(&[chunk(json!({"content":"partial"}), Value::Null)], true),
    ] {
        let (out, _) = run_wire(
            vec![("200 OK", response)],
            &["--write", "inspect"],
            "",
            &ws,
            None,
            |_| {},
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(!ws.0.join("never").exists());
    }
}

#[test]
fn verbose_prints_execution_to_stderr_and_preserves_final_stdout() {
    let ws = Workspace::new();
    let (out, _) = run(
        vec![
            (
                "200 OK",
                tools(&[("ok", "printf tool-output"), ("denied", "touch never")]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--verbose", "--max-steps", "2", "inspect"],
        "",
        &ws,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"done\n");
    let trace = String::from_utf8(out.stderr).unwrap();
    for expected in [
        "model step 1/2",
        "printf tool-output",
        "tool-output",
        "exit=0",
        "permission denied",
        "done",
    ] {
        assert!(trace.contains(expected), "missing {expected:?}: {trace}");
    }
    assert!(!trace.contains("test-key"));
    assert!(!ws.0.join("never").exists());
}

#[test]
fn verbose_stream_text_is_visible_before_upstream_finishes() {
    use std::os::fd::AsRawFd;
    let ws = Workspace::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (release, wait) = std::sync::mpsc::channel();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("accept: {e}"),
            }
        };
        read_request(&mut stream);
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {}\n\n", chunk(json!({"content":"early-token"}), Value::Null)).unwrap();
        stream.flush().unwrap();
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        write!(
            stream,
            "data: {}\n\ndata: [DONE]\n\n",
            chunk(json!({}), json!("stop"))
        )
        .unwrap();
    });
    let mut child = cli()
        .current_dir(&ws.0)
        .env("API_KEY", "test-key")
        .args(["--base-url", &url, "--model", "test-model", "-v", "inspect"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pipe = child.stderr.as_mut().unwrap();
    let fd = pipe.as_raw_fd();
    // SAFETY: fcntl changes the flags of this test-owned pipe and they are restored below.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let started = Instant::now();
    let mut seen = Vec::new();
    while started.elapsed() < Duration::from_secs(2) {
        let mut buf = [0; 1024];
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => seen.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) => panic!("stderr: {e}"),
        }
        if String::from_utf8_lossy(&seen).contains("early-token") {
            break;
        }
    }
    let visible = String::from_utf8_lossy(&seen).contains("early-token");
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, flags);
    }
    let _ = release.send(());
    let out = child.wait_with_output().unwrap();
    server.join().unwrap();
    assert!(
        visible,
        "text was buffered until completion: {}",
        String::from_utf8_lossy(&seen)
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"early-token\n");
}

#[test]
fn verbose_shell_output_is_visible_while_tool_is_running() {
    use std::os::fd::AsRawFd;
    let ws = Workspace::new();
    let mut seen = Vec::new();
    let mut early = false;
    let (out, _) = run_custom(
        vec![
            (
                "200 OK",
                tools(&[("live", "printf tool-early; sleep 2; printf tool-late")]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["-v", "inspect"],
        "",
        &ws,
        None,
        |child| {
            let pipe = child.stderr.as_mut().unwrap();
            let fd = pipe.as_raw_fd();
            // SAFETY: this is a test-owned pipe; its original flags are restored before waiting.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
                0
            );
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(1500) {
                let mut buf = [0; 1024];
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("stderr: {e}"),
                }
                // Look for actual output after the command log, not text inside the logged command.
                if String::from_utf8_lossy(&seen).contains("printf tool-late\ntool-early") {
                    early = true;
                    break;
                }
            }
            unsafe {
                libc::fcntl(fd, libc::F_SETFL, flags);
            }
        },
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        early,
        "tool output was buffered: {}",
        String::from_utf8_lossy(&seen)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("tool-late"));
    assert_eq!(out.stdout, b"done\n");
}

#[test]
fn streaming_budget_and_invalid_tool_index_fail_closed() {
    let ws = Workspace::new();
    let oversized = Reply::Sse(vec![
        format!(":{}\n\n", "x".repeat(4 * 1024 * 1024)).into_bytes(),
    ]);
    let invalid_index = sse(
        &[chunk(
            json!({"tool_calls":[{"index":64,"id":"bad","type":"function","function":{"name":"shell","arguments":"{\"command\":\"touch never\"}"}}]}),
            json!("tool_calls"),
        )],
        true,
    );
    for response in [oversized, invalid_index] {
        let (out, _) = run_wire(
            vec![("200 OK", response)],
            &["--write", "inspect"],
            "",
            &ws,
            None,
            |_| {},
        );
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(!ws.0.join("never").exists());
    }
}

#[test]
fn streaming_preserves_opaque_metadata_for_tool_replay() {
    let ws = Workspace::new();
    let (out, req) = run_wire(
        vec![
            (
                "200 OK",
                sse(
                    &[
                        chunk(
                            json!({"vendor":{"a":1},"tool_calls":[{"index":0,"id":"one","function":{"name":"shell","arguments":"{\"command\":\"printf ok\"}","vendor":{"x":true}},"extra_content":{"google":{"thought_signature":"opaque-token"}}}]}),
                            Value::Null,
                        ),
                        chunk(
                            json!({"vendor":{"b":2},"tool_calls":[{"index":0,"extra_content":{"google":{"thought_signature":"opaque-token"}}}]}),
                            json!("tool_calls"),
                        ),
                    ],
                    true,
                ),
            ),
            ("200 OK", Reply::Json(final_response("done"))),
        ],
        &["inspect"],
        "",
        &ws,
        None,
        |_| {},
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let message = &req[1]["messages"][2];
    assert_eq!(message["vendor"], json!({"a":1,"b":2}));
    assert_eq!(
        message["tool_calls"][0]["extra_content"],
        json!({"google":{"thought_signature":"opaque-token"}})
    );
    assert_eq!(
        message["tool_calls"][0]["function"]["vendor"],
        json!({"x":true})
    );
}

#[test]
fn streaming_accepts_bom_and_all_sse_line_endings() {
    let ws = Workspace::new();
    for ending in ["\r", "\r\n", "\n"] {
        let wire = format!(
            "\u{feff}data: {}{ending}{ending}data: {}{ending}{ending}data: [DONE]{ending}{ending}",
            chunk(json!({"content":"first"}), Value::Null),
            chunk(json!({"content":"second"}), json!("stop"))
        );
        let (out, _) = run_wire(
            vec![(
                "200 OK",
                Reply::Sse(wire.as_bytes().chunks(3).map(<[u8]>::to_vec).collect()),
            )],
            &["inspect"],
            "",
            &ws,
            None,
            |_| {},
        );
        assert!(
            out.status.success(),
            "{ending:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, b"firstsecond\n");
    }
}

#[test]
fn verbose_backpressure_does_not_suspend_shell_timeout() {
    let ws = Workspace::new();
    let mut finished_before_drain = false;
    let (out, req) = run_custom(
        vec![
            ("200 OK", tools(&[("noisy", "yes x")])),
            ("200 OK", final_response("done")),
        ],
        &["-v", "--shell-timeout", "1", "inspect"],
        "",
        &ws,
        None,
        |child| {
            // Leave stderr full until after the command deadline, then drain for safe cleanup.
            thread::sleep(Duration::from_millis(2200));
            finished_before_drain = child.try_wait().unwrap().is_some();
        },
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        finished_before_drain,
        "verbose output blocked the agent until stderr was drained"
    );
    assert_eq!(result(&req[1], 3)["timed_out"], true);
    assert_eq!(out.stdout, b"done\n");
}

#[test]
fn conflicting_stream_metadata_fails_before_tools_execute() {
    let ws = Workspace::new();
    let (out, _) = run_wire(
        vec![(
            "200 OK",
            sse(
                &[
                    chunk(
                        json!({"vendor":{"signature":"one"},"tool_calls":[{"index":0,"id":"write","function":{"name":"shell","arguments":"{\"command\":\"touch never\"}"}}]}),
                        Value::Null,
                    ),
                    chunk(json!({"vendor":{"signature":"two"}}), json!("tool_calls")),
                ],
                true,
            ),
        )],
        &["--write", "inspect"],
        "",
        &ws,
        None,
        |_| {},
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("conflicting model stream metadata"));
    assert!(out.stdout.is_empty());
    assert!(!ws.0.join("never").exists());
}

#[test]
fn configuration_reads_simple_environment_names_and_ignores_legacy_names() {
    let ws = Workspace::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request(&mut stream);
                    let body = final_response("done").to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    return Some(request);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("accept: {e}"),
            }
        }
    });
    let out = cli()
        .current_dir(&ws.0)
        .env("BASE_URL", url)
        .env("MODEL", "simple-model")
        .env("API_KEY", "test-key")
        .env("MA_BASE_URL", "invalid-old-url")
        .env("MA_MODEL", "old-model")
        .env("MA_API_KEY", "old-key")
        .env("OPENAI_BASE_URL", "invalid-old-url")
        .env("OPENAI_MODEL", "old-model")
        .env("OPENAI_API_KEY", "old-key")
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let request = server.join().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"done\n");
    assert_eq!(request.unwrap()["model"], "simple-model");
    let old = cli()
        .env("MA_MODEL", "old-model")
        .env("MA_API_KEY", "old-key")
        .env("OPENAI_MODEL", "old-model")
        .env("OPENAI_API_KEY", "old-key")
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(old.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&old.stderr).contains("API_KEY"));
}

fn context_error() -> Value {
    json!({"error":{"code":"context_length_exceeded","message":"Too many input tokens"}})
}

#[test]
fn context_limit_trims_complete_old_turns_without_reexecuting_shell() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("old-one", "printf x >> count"),
                    ("old-two", "printf older"),
                ]),
            ),
            ("200 OK", tools(&[("recent", "printf y >> count")])),
            ("400 Bad Request", context_error()),
            ("200 OK", final_response("done")),
        ],
        &["-v", "--write", "--max-steps", "4", "original task"],
        "",
        &ws,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"done\n");
    assert_eq!(std::fs::read(ws.0.join("count")).unwrap(), b"xy");
    assert_eq!(req.len(), 4);
    assert_eq!(req[3]["messages"][0], req[2]["messages"][0]);
    assert_eq!(req[3]["messages"][1], req[2]["messages"][1]);
    let retry = req[3]["messages"].as_array().unwrap();
    assert_eq!(retry.len(), 5);
    assert_eq!(retry[2]["role"], "user");
    assert!(retry[2]["content"].as_str().unwrap().contains("removed"));
    assert_eq!(retry[3], req[2]["messages"][5]);
    assert_eq!(retry[4], req[2]["messages"][6]);
    assert_eq!(req[3]["tools"], req[2]["tools"]);
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(log.contains("context too long"));
    assert!(log.contains("removed 1 old turn"));
}

#[test]
fn context_recovery_preserves_recent_multi_tool_turn_and_can_recur() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            ("200 OK", tools(&[("old", "printf old")])),
            (
                "200 OK",
                tools(&[("one", "printf one"), ("two", "printf two")]),
            ),
            ("400 Bad Request", context_error()),
            ("200 OK", tools(&[("new", "printf new")])),
            ("400 Bad Request", context_error()),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(req.len(), 6);
    let first_retry = req[3]["messages"].as_array().unwrap();
    assert_eq!(
        &first_retry[3..],
        &req[2]["messages"].as_array().unwrap()[4..]
    );
    assert_eq!(first_retry[3]["tool_calls"].as_array().unwrap().len(), 2);
    assert_eq!(first_retry[4]["tool_call_id"], "one");
    assert_eq!(first_retry[5]["tool_call_id"], "two");
    let next_retry = req[5]["messages"].as_array().unwrap();
    assert_eq!(next_retry.len(), 5);
    assert_eq!(next_retry[3]["tool_calls"][0]["id"], "new");
    assert_eq!(next_retry[4]["tool_call_id"], "new");
    assert_eq!(out.stdout, b"done\n");
    assert!(out.stderr.is_empty());
}

#[test]
fn context_recovery_retries_once_and_other_http_errors_never_trim() {
    let ws = Workspace::new();
    for (status, error, retries) in [
        ("400 Bad Request", context_error(), 1),
        ("413 Payload Too Large", context_error(), 1),
        (
            "400 Bad Request",
            json!({"error":{"code":"invalid_request_error","message":"invalid tools"}}),
            0,
        ),
        (
            "400 Bad Request",
            json!({"error":{"code":"rate_limit_exceeded","message":"token limit exceeded"}}),
            0,
        ),
        ("401 Unauthorized", context_error(), 0),
        ("429 Too Many Requests", context_error(), 0),
        ("500 Internal Server Error", context_error(), 0),
    ] {
        let mut replies = vec![
            ("200 OK", tools(&[("old", "printf old")])),
            ("200 OK", tools(&[("recent", "printf recent")])),
            (status, error.clone()),
        ];
        if retries > 0 {
            replies.push((status, error));
        }
        let (out, req) = run(replies, &["inspect"], "", &ws);
        assert_eq!(out.status.code(), Some(1), "{status}");
        assert!(out.stdout.is_empty());
        assert_eq!(req.len(), 3 + retries, "{status}");
        if retries > 0 {
            assert!(String::from_utf8_lossy(&out.stderr).contains("after history trimming"));
        } else {
            assert!(
                String::from_utf8_lossy(&out.stderr).contains(status.split(' ').next().unwrap())
            );
        }
    }
}

#[test]
fn context_error_without_old_turns_exits_clearly() {
    let ws = Workspace::new();
    for turns in 0..=1 {
        let mut replies = Vec::new();
        if turns == 1 {
            replies.push(("200 OK", tools(&[("recent", "printf recent")])));
        }
        replies.push(("400 Bad Request", context_error()));
        let (out, req) = run(replies, &["inspect"], "", &ws);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert_eq!(req.len(), turns + 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("no old complete turns"));
    }
}

#[test]
fn context_retry_counts_toward_max_steps_and_never_executes_last_tools() {
    let ws = Workspace::new();
    for max in [3, 4] {
        let mut replies = vec![
            ("200 OK", tools(&[("old", "printf old")])),
            ("200 OK", tools(&[("recent", "printf recent")])),
            ("400 Bad Request", context_error()),
        ];
        if max == 4 {
            replies.push(("200 OK", tools(&[("never", "touch never")])));
        }
        let (out, req) = run(
            replies,
            &["--write", "--max-steps", &max.to_string(), "inspect"],
            "",
            &ws,
        );
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(req.len(), max);
        assert!(out.stdout.is_empty());
        assert!(!ws.0.join("never").exists());
    }
}

#[test]
fn context_error_detection_requires_explicit_upstream_signal() {
    let ws = Workspace::new();
    for error in [
        json!({"error":{"type":"context_window_exceeded"}}),
        json!({"error":{"message":"This model's maximum context length is 8192 tokens. However, you requested 9000 tokens."}}),
        json!({"error":{"message":"Input exceeds the context window"}}),
    ] {
        let (out, _) = run(vec![("400 Bad Request", error)], &["inspect"], "", &ws);
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("no old complete turns"));
    }
}

#[test]
fn context_stream_error_recovers_without_executing_partial_tool_calls() {
    let ws = Workspace::new();
    let (out, req) = run_wire(
        vec![
            ("200 OK", Reply::Json(tools(&[("old", "printf old")]))),
            ("200 OK", Reply::Json(tools(&[("recent", "printf recent")]))),
            (
                "200 OK",
                sse(
                    &[
                        chunk(
                            json!({"tool_calls":[{"index":0,"id":"never","function":{"name":"shell","arguments":"{\"command\":\"touch never\"}"}}]}),
                            Value::Null,
                        ),
                        context_error(),
                    ],
                    false,
                ),
            ),
            (
                "200 OK",
                sse(&[chunk(json!({"content":"done"}), json!("stop"))], true),
            ),
        ],
        &["--write", "inspect"],
        "",
        &ws,
        None,
        |_| {},
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(req.len(), 4);
    assert_eq!(out.stdout, b"done\n");
    assert!(!ws.0.join("never").exists());
}

fn write_config(workspace: &Workspace, body: &str, private: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = workspace.0.join("cfg");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, body).unwrap();
    let mode = if private { 0o600 } else { 0o644 };
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn config_file_supplies_connection_settings() {
    let ws = Workspace::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = serve(
        listener,
        vec![("200 OK", Reply::Json(final_response("done")))],
    );
    let config = write_config(
        &ws,
        &format!(r#"{{"base_url":"{url}","api_key":"test-key","model":"config-model"}}"#),
        true,
    );
    // Without endpoint flags or environment, a served request can only come from the file.
    let out = cli()
        .current_dir(&ws.0)
        .env("MA_CONFIG", &config)
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let requests = server.join().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(requests[0]["model"], "config-model");
    assert_eq!(out.stdout, b"done\n");
}

#[test]
fn environment_overrides_config_file() {
    let ws = Workspace::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = serve(
        listener,
        vec![("200 OK", Reply::Json(final_response("done")))],
    );
    // The config points at a dead port; only the environment can make this test reach the mock.
    let config = write_config(
        &ws,
        r#"{"base_url":"http://127.0.0.1:1/v1","api_key":"test-key","model":"config-model"}"#,
        true,
    );
    let out = cli()
        .current_dir(&ws.0)
        .env("MA_CONFIG", &config)
        .env("BASE_URL", &url)
        .env("MODEL", "env-model")
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let requests = server.join().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(requests[0]["model"], "env-model");
}

#[test]
fn config_file_rejects_permissions_unknown_keys_and_shapes() {
    let ws = Workspace::new();
    for (body, needle) in [
        (r#"{"write":true}"#, "permissions are not configurable"),
        (r#"{"net":true}"#, "permissions are not configurable"),
        (r#"{"baseUrl":"x"}"#, "unknown key"),
        (r#"{"max_steps":5}"#, "unknown key"),
        (r#"{"model":42}"#, "must be a string"),
        (r#"["x"]"#, "must be a JSON object"),
        (r#"{"model":"m""#, "config"),
    ] {
        let config = write_config(&ws, body, true);
        let out = cli()
            .current_dir(&ws.0)
            .env("MA_CONFIG", &config)
            .arg("inspect")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{body}: {stderr}");
        assert!(out.stdout.is_empty(), "{body}");
        assert!(stderr.contains(needle), "{body} -> {stderr}");
    }
}

#[test]
fn config_file_with_api_key_must_be_private() {
    let ws = Workspace::new();
    let config = write_config(&ws, r#"{"api_key":"test-key"}"#, false);
    let out = cli()
        .current_dir(&ws.0)
        .env("MA_CONFIG", &config)
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("chmod 600"), "{stderr}");
    // The same content is accepted once the mode is private; it then fails on the missing model.
    let config = write_config(&ws, r#"{"api_key":"test-key"}"#, true);
    let out = cli()
        .current_dir(&ws.0)
        .env("MA_CONFIG", &config)
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(!stderr.contains("chmod 600"), "{stderr}");
    assert!(stderr.contains("MODEL"), "{stderr}");
}

#[test]
fn missing_explicit_config_path_is_an_error() {
    let ws = Workspace::new();
    let out = cli()
        .current_dir(&ws.0)
        .env("MA_CONFIG", ws.0.join("absent.json"))
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("config"), "{stderr}");
}

#[test]
fn readonly_globs_expand_inside_workspace() {
    let ws = Workspace::new();
    std::fs::write(ws.0.join("a.rs"), "alpha\n").unwrap();
    std::fs::write(ws.0.join("b.rs"), "beta\n").unwrap();
    std::fs::create_dir(ws.0.join("src")).unwrap();
    std::fs::write(ws.0.join("src/c.rs"), "nested\n").unwrap();
    let commands = [
        ("cat", "cat *.rs"),
        ("wc", "wc -l *.rs"),
        ("ls", "ls ./*.rs"),
        ("nested", "cat */*.rs"),
        ("bracket", "cat [ab].rs"),
        ("question", "cat ?.rs"),
    ];
    let (out, req) = run(
        vec![
            ("200 OK", tools(&commands)),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    for index in 3..9 {
        let tool = result(&req[1], index);
        assert!(
            tool.get("error").is_none(),
            "{}: {tool}",
            commands[index - 3].1
        );
        assert_eq!(tool["exit_code"], 0);
    }
    assert_eq!(result(&req[1], 3)["stdout"], "alpha\nbeta\n");
    assert_eq!(result(&req[1], 6)["stdout"], "nested\n");
}

#[test]
fn readonly_globs_keep_path_boundaries_and_writes_literal() {
    use std::os::unix::fs::symlink;
    let ws = Workspace::new();
    let outside = Workspace::new();
    std::fs::write(outside.0.join("secret.rs"), "secret").unwrap();
    symlink(outside.0.join("secret.rs"), ws.0.join("escape.rs")).unwrap();
    symlink(&outside.0, ws.0.join("escape-dir")).unwrap();
    let commands = [
        ("link", "cat *.rs"),
        ("directory", "cat */*.rs"),
        ("parent", "cat ../*.rs"),
        ("dots", "cat .*/secret.rs"),
        ("write", "rm *.rs"),
        ("redirect", "printf nope > *.rs"),
    ];
    let (out, req) = run(
        vec![
            ("200 OK", tools(&commands)),
            ("200 OK", final_response("done")),
        ],
        &["--write", "inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    for index in 3..9 {
        let tool = result(&req[1], index);
        assert!(
            tool["error"]
                .as_str()
                .is_some_and(|e| e.contains("permission denied")),
            "{}: {tool}",
            commands[index - 3].1
        );
        assert_eq!(tool["stdout"], "");
    }
    assert_eq!(
        std::fs::read_to_string(outside.0.join("secret.rs")).unwrap(),
        "secret"
    );
    assert!(ws.0.join("escape.rs").is_symlink());
}

#[test]
fn readonly_glob_matching_respects_quoted_metacharacters() {
    use std::os::unix::fs::symlink;
    let ws = Workspace::new();
    let outside = Workspace::new();
    std::fs::write(ws.0.join("a[1].rs"), "literal\n").unwrap();
    std::fs::write(outside.0.join("secret.rs"), "secret").unwrap();
    symlink(outside.0.join("secret.rs"), ws.0.join("b[1].rs")).unwrap();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[("literal", "cat 'a[1]'*.rs"), ("escape", "cat 'b[1]'*.rs")]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(result(&req[1], 3)["stdout"], "literal\n");
    assert!(
        result(&req[1], 4)["error"]
            .as_str()
            .unwrap()
            .contains("outside workspace")
    );
}

#[test]
fn readonly_globs_check_shell_quote_rules_and_option_separator() {
    use std::os::unix::fs::symlink;
    let ws = Workspace::new();
    let outside = Workspace::new();
    std::fs::write(outside.0.join("secret"), "secret").unwrap();
    for name in ["a.rs", "].rs", "a\\q.rs", "a\\*.rs", "-escape.rs", "你.rs"] {
        symlink(outside.0.join("secret"), ws.0.join(name)).unwrap();
    }
    std::fs::create_dir(ws.0.join("local:")).unwrap();
    symlink(outside.0.join("secret"), ws.0.join("local:/escape.rs")).unwrap();
    let commands = [
        ("negation", "cat [\"!\"a].rs"),
        ("closing", "cat [a\"]\"].rs"),
        ("backslash", r#"cat "a\q"*.rs"#),
        ("star", r#"cat "a\*"*.rs"#),
        ("option", "cat -- -*.rs"),
        ("option-literal", "cat -- -escape.rs"),
        ("wc", "wc -l -- -*.rs"),
        ("unicode", "cat ?.rs"),
        ("colon", "cat local://*.rs"),
    ];
    let (out, req) = run_with_env(
        vec![
            ("200 OK", tools(&commands)),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
        Some(("LC_ALL", std::ffi::OsStr::new("C.UTF-8"))),
    );
    assert!(out.status.success());
    for (index, (_, command)) in commands.iter().enumerate() {
        let tool = result(&req[1], index + 3);
        assert!(
            tool["error"]
                .as_str()
                .is_some_and(|e| e.contains("permission denied")),
            "{command}: {tool}"
        );
        assert_eq!(tool["stdout"], "", "{command}");
    }
}

#[test]
fn readonly_question_glob_matches_unicode_names() {
    let ws = Workspace::new();
    std::fs::write(ws.0.join("你.rs"), "unicode\n").unwrap();
    let (out, req) = run_with_env(
        vec![
            ("200 OK", tools(&[("unicode", "cat ?.rs")])),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
        Some(("LC_ALL", std::ffi::OsStr::new("C.UTF-8"))),
    );
    assert!(out.status.success());
    assert_eq!(result(&req[1], 3)["stdout"], "unicode\n");
    assert_eq!(result(&req[1], 3)["exit_code"], 0);
    let outside = Workspace::new();
    std::fs::write(outside.0.join("secret"), "secret").unwrap();
    std::fs::remove_file(ws.0.join("你.rs")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("secret"), ws.0.join("你.rs")).unwrap();
    let (_, req) = run_with_env(
        vec![
            ("200 OK", tools(&[("unicode", "cat ?.rs")])),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
        Some(("LC_ALL", std::ffi::OsStr::new("C.UTF-8"))),
    );
    assert!(
        result(&req[1], 3)["error"]
            .as_str()
            .is_some_and(|e| e.contains("outside workspace"))
    );
    assert_eq!(result(&req[1], 3)["stdout"], "");
}

#[test]
#[cfg(target_os = "macos")]
fn invalid_locale_cannot_silently_change_guard_glob_matching() {
    let output = cli()
        .env_remove("LC_ALL")
        .env("LANG", "C.UTF-8")
        .env("LC_TIME", "ma_invalid_locale")
        .arg("inspect")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid locale"));
    let help = cli()
        .env("LC_ALL", "ma_invalid_locale")
        .args(["prompt", "--help"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"ma [OPTIONS]"));
}

#[test]
fn readonly_globs_check_literal_fallback_and_quoted_collation() {
    use std::os::unix::fs::symlink;
    let ws = Workspace::new();
    let outside = Workspace::new();
    std::fs::write(outside.0.join("secret.rs"), "secret").unwrap();
    symlink(outside.0.join("secret.rs"), ws.0.join("[ab].rs")).unwrap();
    symlink(&outside.0, ws.0.join("[ab]")).unwrap();
    symlink(outside.0.join("secret.rs"), ws.0.join("a].rs")).unwrap();
    std::fs::create_dir(ws.0.join("a")).unwrap();
    std::fs::write(ws.0.join("b"), "file").unwrap();
    symlink(&outside.0, ws.0.join("[bc]")).unwrap();
    let commands = [
        ("literal", "cat [ab].rs"),
        ("directory", "cat [ab]/secret.rs"),
        ("collation", r#"cat [["."a"."]].rs"#),
        ("leading-close", r#"cat []"!"a].rs"#),
        ("slash", "ls [bc]/"),
        ("dot", "ls [bc]/."),
        ("parent", "ls [bc]/.."),
    ];
    let (out, req) = run(
        vec![
            ("200 OK", tools(&commands)),
            ("200 OK", final_response("done")),
        ],
        &["inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    for (index, (_, command)) in commands.iter().enumerate() {
        let tool = result(&req[1], index + 3);
        assert!(
            tool["error"]
                .as_str()
                .is_some_and(|e| e.contains("permission denied")),
            "{command}: {tool}"
        );
        assert_eq!(tool["stdout"], "");
    }
}

#[test]
fn literal_braces_are_not_shell_grouping_and_find_diagnostic_is_precise() {
    let ws = Workspace::new();
    let (out, req) = run(
        vec![
            (
                "200 OK",
                tools(&[
                    ("literal", "printf '%s' {}"),
                    ("find", "find . -exec cat {} +"),
                    ("group", "{ touch never; }"),
                ]),
            ),
            ("200 OK", final_response("done")),
        ],
        &["--write", "inspect"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(result(&req[1], 3)["stdout"], "{}");
    let find = result(&req[1], 4);
    let error = find["error"].as_str().unwrap();
    assert!(error.contains("find") && error.contains("-exec"), "{error}");
    assert!(!error.contains("grouping"), "{error}");
    assert!(
        result(&req[1], 5)["error"]
            .as_str()
            .unwrap()
            .contains("grouping")
    );
    assert!(!ws.0.join("never").exists());
}

#[test]
fn help_and_version_work_after_prompt_but_not_as_values_or_after_separator() {
    for flag in ["--help", "-h", "--version"] {
        let out = cli()
            .env("MA_CONFIG", "/missing/ma-config.json")
            .args(["prompt", flag])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{flag}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stdout).starts_with(if flag == "--version" {
                "ma 0."
            } else {
                "ma [OPTIONS]"
            })
        );
        let value = cli()
            .args(["--model", flag, "--base-url", "invalid", "prompt"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(value.status.code(), Some(2));
        assert!(value.stdout.is_empty());
    }
    let ws = Workspace::new();
    let (out, req) = run(
        vec![("200 OK", final_response("done"))],
        &["--", "--help", "--version"],
        "",
        &ws,
    );
    assert!(out.status.success());
    assert_eq!(out.stdout, b"done\n");
    assert_eq!(req[0]["messages"][1]["content"], "--help --version");
}
