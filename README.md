# Minimal Agent

**One model. One tool. One loop.**

> **English TL;DR** — `ma` is a one-shot, single-binary coding agent in Rust: one model (OpenAI-compatible Chat Completions), one tool (`shell`), one loop. No runtime dependencies, no sessions, no plugins, ~1.3 MB release binary. Permissions (`--write` / `--net`) are fixed at startup, and the built-in Guard is a best-effort gate — **not a sandbox** ([SECURITY.md](SECURITY.md)).

`ma` 是一次性运行的 Rust Agent executable。通过 OpenAI-compatible Chat Completions 请求一个模型，只暴露 `shell(command)`，输出最终回答后退出。能力来自宿主 CLI。

## 构建与调用

V0 支持 macOS / Linux，构建需要 Rust 1.85+ 和 C 编译器；运行无需 Rust、Python、Node 或 JVM。HTTP/TLS 使用同步 [ureq](https://docs.rs/ureq/3.4.2/ureq/)，不依赖宿主 curl 或 OpenSSL。工具命令使用系统 `/bin/sh`。

```sh
cargo build --release --locked

export MA_BASE_URL='https://your-endpoint.example/v1'
export MA_MODEL='your-model'
export MA_API_KEY='your-key'

./target/release/ma '分析当前项目'
git diff | ./target/release/ma '总结这些修改'
./target/release/ma --write '修复当前目录里的配置文件'
./target/release/ma --net '检查这个依赖的问题'
./target/release/ma --write --net '升级项目依赖'
./target/release/ma --verbose --write '修复配置文件'
```

也支持 `--base-url`、`--model`、`--api-key` 参数（密钥建议通过环境变量传递）。`OPENAI_BASE_URL`、`OPENAI_MODEL`、`OPENAI_API_KEY` 是对应环境变量的后备；`MA_*` 优先，参数优先于环境变量。默认 API 根目录为 `https://api.openai.com/v1`，客户端追加 `/chat/completions`，不要传入完整接口路径。

Prompt 可为一个或多个位置参数；管道 stdin 提供补充上下文，也可以只从 stdin 提供任务。输入必须是 UTF-8，合并后最多 1 MiB。Prompt 以 `-` 开头时使用 `--` 分隔。

```sh
printf '%s' '列出当前目录中的 Rust 文件' | ./target/release/ma
./target/release/ma --max-steps 10 --shell-timeout 15 --http-timeout 90 '分析构建失败原因'
./target/release/ma --help
```

## 流式响应与执行过程

默认向上游发送 `stream: true`，同步读取 Chat Completions SSE，不需要异步运行时。模型文本和工具参数按分片组装，工具调用必须完整收到 `finish_reason` 与 `[DONE]` 后才执行；断流、畸形事件和超出预算会按错误退出。直接返回 JSON 的 endpoint 仍可兼容。

默认只在 stdout 输出完整最终回答。使用 `--verbose` / `-v` 开启实时过程：模型请求轮次和耗时、模型文本片段、shell 命令、工具 stdout/stderr、退出码、拒绝、超时和截断标记均写到 stderr。工具输出在执行中排空时展示，不等命令结束；两路工具输出在过程日志中混合展示，返回模型时仍分别保留。授权头和 API key 配置不会写入过程日志；日志中的模型与工具文本仍是任务原始数据。

```sh
./target/release/ma -v '分析当前项目'
./target/release/ma -v --write '修复配置文件' > answer.txt
# 最终回答保存在 answer.txt，终端仍实时显示 stderr 中的执行过程
```

verbose 会在 stderr 显示模型文本片段，最后在 stdout 输出一次完整最终回答；程序调用方应分别消费这两路输出。stderr 管道写满时丢弃可选过程日志，避免阻塞模型读取和工具超时；返回模型的工具结果仍按原有预算保留。不输出模型的 reasoning_content 文本，但保留该兼容字段和不冲突的扩展元数据供后续模型请求使用。SSE 支持 LF、CRLF、CR 换行及流开头的 UTF-8 BOM。没有进度 UI、交互式审批或额外日志框架。

## 权限

权限在进程启动时授予，运行中不询问、不升级；拒绝作为工具结果反馈给模型。

| 启动方式 | 工作目录数据读取 | 写权限范围 | shell 网络 |
| --- | --- | --- | --- |
| 默认 | 允许 | 拒绝 | 拒绝 |
| `--write` | 允许 | 启动时 cwd 及其子目录 | 拒绝 |
| `--net` | 允许 | 拒绝 | 允许 |
| `--write --net` | 允许 | 启动时 cwd 及其子目录 | 允许 |

模型 API 请求始终允许，与 shell 的 `--net` 分开。系统可执行文件和运行所需的系统资源仍可使用。V0 不支持配置文件或额外授权目录。

同一份 Policy 生成模型指令并执行 Guard 检查。Guard 拦截常见写命令、文件重定向、明显网络命令；检查文字路径中的父目录、已有符号链接和常见输出选项。`/dev/null` 是可用输出目标，文件描述符重定向如 `>&2` 可用。

**Guard 是 best-effort safety gate，不是安全沙箱。** 未知 CLI、程序内部写入/网络、构建脚本、Git hooks、配置影响和检查后路径变化都可能产生未被识别的副作用；这些能力标志不能提供 OS 强制隔离。运行模型生成的命令时应使用信任的环境，强隔离留给后续 OS Sandbox。

为保持 Guard 简单，V0 拒绝 shell 展开、命令替换、嵌套 shell、控制流、环境前缀、提权、`ln` 和 `xargs` / `find -exec` 等命令转发。`cd` 不与 pipeline、后台分支或 `||` 混用；清除 `CDPATH`，防止环境改变实际工作目录。写路径必须是文字路径，不支持写目标通配符。下载写入使用显式输出路径；`curl -O` 等隐式文件名和无法检查的复合写选项会被拒绝。复杂命令可拆成多次 shell 调用，文本模式和脚本参数可使用单引号。包管理器和构建工具保守地视为写操作；支持的离线标志（例如 `cargo --offline`、`mvn -o`）可免除明显网络检查。其他复杂语法可能误拒绝，由模型调整命令。

## 调用契约

- stdout：仅完整最终文本回答，末尾追加换行；流式中间文本不写入此通道。
- stderr：参数、输入、HTTP、协议及循环终止错误；`--verbose` 开启时额外实时输出模型文本和执行过程。
- 退出码：`0` 最终回答成功；`1` 运行/HTTP/协议错误；`2` 参数/输入错误；`3` 步数耗尽。`0` 表示取得 final，不保证模型声称的任务成功。
- `--max-steps` 默认 `200`，计算模型请求次数。最后一步仍请求模型，但不执行无法再反馈结果的工具调用。
- `--http-timeout` 默认 `120` 秒，`--shell-timeout` 默认 `30` 秒；正整数，最多 `86400`。
- 每次响应（含完整 SSE 流）最多 4 MiB、最多 64 个工具调用；工具按顺序执行。HTTP timeout 限制整个响应时间，包括流式读取。
- shell stdin 关闭，不支持交互命令。每次调用从启动工作目录执行，`cd` 不跨工具调用保留。
- stdout/stderr 分别保留前 64 KiB，多余内容继续排空但不保存，结果包含 `truncated`。
- Unix shell 在独立进程组中运行；超时杀掉该组，shell 结束时清理残留组成员。Agent 收到 SIGINT、SIGTERM、SIGHUP 时杀掉当前工具进程组，按 `128 + signal` 退出。SIGKILL 和主动脱离进程组的程序不属于该清理机制的强保证范围。
- `MA_API_KEY`、`OPENAI_API_KEY` 不传给 shell 子进程；其他宿主环境仍继承。
- HTTP 请求不自动重试、不跟随重定向；TLS 验证使用依赖内置的根证书，不关闭证书校验。

工具结果是 JSON 字符串，包含 `stdout`、`stderr`、`exit_code`、`timed_out`、`truncated`；拒绝或执行错误另含 `error`。模型协议遵循 [Chat Completions 的 function calling](https://developers.openai.com/api/docs/guides/function-calling)。assistant 消息与工具 ID 保留；空 final、截断完成和畸形协议按错误退出。

## 验证

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
wc -c target/release/ma
```

测试使用临时工作目录、真实 `/bin/sh` 和本地 mock HTTP 服务，无需密钥或真实模型。CI 对 macOS / Linux 执行相同检查，并要求 release binary 小于 10,000,000 字节；目标小于 5,000,000 字节。`rg`、`fd`、`jq` 等是可选宿主工具，不是 `ma` 运行依赖。

设计及开发验收记录见 [docs/design.md](docs/design.md)。无 Memory、Session、MCP、Skills、Planner、Sub-Agent、Provider abstraction、GUI 或 Daemon。

## License

[MIT](LICENSE)。
