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
        "MA_BASE_URL",
        "MA_API_KEY",
        "MA_MODEL",
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_MODEL",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        c.env_remove(name);
    }
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

fn run_wire(
    responses: Vec<(&'static str, Reply)>,
    flags: &[&str],
    input: &str,
    workspace: &Workspace,
    env: Option<(&str, &std::ffi::OsStr)>,
    after_spawn: impl FnOnce(&mut std::process::Child),
) -> (Output, Vec<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
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
    });
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
        .env("MA_API_KEY", "test-key")
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
                    ("secret", "printenv MA_API_KEY"),
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
        .env("MA_API_KEY", "test-key")
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
        .env("MA_API_KEY", "test-key")
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
