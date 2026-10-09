# 使用细节

安装与日常用法见 [README](../README.md)。以下示例假定 `ma` 已安装并在 `PATH` 中。

## 只构建，不安装

如果只想在源码目录构建：

```sh
cargo build --release --locked
./target/release/ma --help
```

产物是 `target/release/ma`。`./target/release/ma` 是相对源码目录的路径；切换到其他项目后，需要用二进制的绝对路径，或先将它安装到 `PATH`：

```sh
cd /path/to/your/project
/path/to/mini-agent/target/release/ma -v '分析当前项目'
```

将示例中的两个路径替换为实际目录。工作目录取启动位置，与二进制放在哪里无关。

安装到用户级 `PATH` 用仓库内的脚本：

```sh
./scripts/install.sh              # 安装到 ~/.local/bin
./scripts/install.sh --prefix ~/.cargo   # 或任意根目录，二进制落在 <DIR>/bin
./scripts/install.sh --uninstall  # 卸载
```

脚本默认安装到 `~/.local/bin`，该目录通常在 `PATH` 上排在 `~/.cargo/bin` 之前，因此后来的同名命令不会被旧副本静默遮蔽。安装后若发现旧副本仍由 cargo 管理，脚本会把它移除；不是 cargo 管理的文件不会被删除。`--offline` 传给 cargo 以禁止联网（依赖必须已在本地缓存中）。

## 更新

`ma --update` 下载最新发布版中与本平台匹配的产物并原地替换自身，只接受 `ma-<版本>-<target>.tar.gz` 里的 `ma`，其他成员和越出归档根的路径一律拒绝。

替换前会先把候选二进制写到目标同目录的临时文件并执行 `--version`，**只有当它报出的版本等于 API 公布的版本时才替换**。截断的下载、错架构的产物、被保存成文件的 HTML 错误页都无法覆盖一个可用的 `ma`；任何一步失败都会清理临时文件并保留原二进制。若二进制是通过符号链接调用的（例如 `~/.local/bin/ma -> versions/.../ma`），替换的是链接指向的目标，链接本身不动。

普通运行还会每天检查一次新版本，发现更新时向 stderr 打印一行提示（不经 `--verbose`、不写 stdout，因此管道输出仍是纯 final）。检查是尽力而为：任何错误静默忽略，时间戳记录在 `~/.config/ma/last-update-check`，24 小时内不重复请求。`--no-update-check` 完全关闭。

## 参数

| 参数 | 说明 | 默认值 |
| --- | --- | --- |
| `--base-url URL` | 模型 API 根地址 | `https://api.openai.com/v1` |
| `--model MODEL` | 模型名称 | 环境变量或配置文件；必需 |
| `--api-key KEY` | API 密钥，建议用环境变量传入 | 环境变量或配置文件；必需 |
| `--skills NAMES` | 按目录名加载 skills，多个名称用英文逗号分隔 | 不加载 |
| `--write` | 允许写入启动时的当前目录及其子目录 | 关闭 |
| `--net` | 允许 shell 联网 | 关闭 |
| `--update` | 用最新发布版替换当前二进制，需单独使用 | — |
| `--no-update-check` | 跳过每天一次的新版本检查 | 检查开启 |
| `-v, --verbose` | 实时显示执行过程 | 关闭 |
| `--max-steps N` | 最多请求模型的次数 | `200` |
| `--http-timeout SEC` | 单次模型请求超时，秒 | `120` |
| `--shell-timeout SEC` | 单次 shell 命令超时，秒 | `30` |
| `-h, --help` | 显示帮助 | — |
| `--version` | 显示版本 | — |
| `--` | 将其后的参数全部视为 Prompt | — |

## 配置与输入

使用 `BASE_URL`、`MODEL`、`API_KEY` 配置模型，也支持 `--base-url`、`--model`、`--api-key` 参数（密钥建议通过环境变量传递）。模型连接设置只读取这三个环境变量。默认 API 根目录为 `https://api.openai.com/v1`，客户端追加 `/chat/completions`，不要传入完整接口路径。

优先级：命令行参数 > 环境变量 > 配置文件。配置文件默认位于 `~/.config/ma/config.json`（遵循 `XDG_CONFIG_HOME`），可用 `MA_CONFIG` 指向其他路径；显式路径不存在会直接报错。文件必须是 JSON 对象，只接受 `base_url`、`api_key`、`model` 三个字符串键；未知键、非字符串值或非对象都会报错，不静默忽略。**权限不能写入配置文件**：`write` / `net` 会被拒绝，权限只能由命令行授予。含 `api_key` 的配置文件必须不能被同组或其他用户读取，否则拒绝启动并提示 `chmod 600`。

例如，创建默认路径的配置文件（使用 `XDG_CONFIG_HOME` 时相应替换路径）：

```sh
mkdir -p ~/.config/ma
```

将以下内容保存为 `~/.config/ma/config.json`，替换为实际连接设置：

```json
{
  "base_url": "https://your-endpoint.example/v1",
  "api_key": "your-key",
  "model": "your-model"
}
```

```sh
chmod 600 ~/.config/ma/config.json
```

Prompt 可为一个或多个位置参数；管道 stdin 提供补充上下文，也可以只从 stdin 提供任务。输入必须是 UTF-8，合并后最多 1 MiB。Prompt 以 `-` 开头时使用 `--` 分隔。

`--help` / `-h` / `--version` 可放在 Prompt 后，例如 `ma '任务' --help`，且无需有效模型配置。其他选项的值不会被识别为帮助标志；`--` 后的内容全部作为 Prompt。

```sh
printf '%s' '列出当前目录中的 Rust 文件' | ma
ma --max-steps 10 --shell-timeout 15 --http-timeout 90 '分析构建失败原因'
ma --help
```

## 流式响应与执行过程

默认向上游发送 `stream: true`，同步读取 Chat Completions SSE，不需要异步运行时。模型文本和工具参数按分片组装，工具调用必须完整收到 `finish_reason` 与 `[DONE]` 后才执行；断流、畸形事件和超出预算会按错误退出。直接返回 JSON 的 endpoint 仍可兼容。

默认只在 stdout 输出完整最终回答。使用 `--verbose` / `-v` 开启实时过程：模型请求轮次和耗时、模型文本片段、shell 命令、工具 stdout/stderr、退出码、拒绝、超时和截断标记均写到 stderr。工具输出在执行中排空时展示，不等命令结束；两路工具输出在过程日志中混合展示，返回模型时仍分别保留。授权头和 API key 配置不会写入过程日志；日志中的模型与工具文本仍是任务原始数据。

```sh
ma -v '分析当前项目'
ma -v --write '修复配置文件' > answer.txt
# 最终回答保存在 answer.txt，终端仍实时显示 stderr 中的执行过程
```

verbose 会在 stderr 显示模型文本片段，最后在 stdout 输出一次完整最终回答；程序调用方应分别消费这两路输出。stderr 管道写满时丢弃可选过程日志，避免阻塞模型读取和工具超时；返回模型的工具结果仍按原有预算保留。不输出模型的 reasoning_content 文本，但保留该兼容字段和不冲突的扩展元数据供后续模型请求使用。SSE 支持 LF、CRLF、CR 换行及流开头的 UTF-8 BOM。没有进度 UI、交互式审批或额外日志框架。

## Skills

```sh
ma --skills code-review,explain '评审并解释这些修改'
ma --skills=code-review '评审这些修改'
```

每个名称按顺序查找以下位置的 `SKILL.md`，使用第一个成功读取的文件：

1. `<启动 cwd>/.agents/skills/<名称>/SKILL.md`
2. `$HOME/.agents/skills/<名称>/SKILL.md`

名称是 skill **目录名**，不按 frontmatter 的 `name` 搜索；允许英文字母、数字、连字符和下划线，不能以连字符开头，也不能传路径。英文逗号分隔的名称会去除首尾空白、忽略空项；可重复传 `--skills`，重复名称或指向同一个实际目录的 skill 只加载一次，保留选择顺序。不传参数就不加载 skills，不扫描其他 Agent 的私有目录，也不向上查找父项目目录。未设置 `HOME` 时只查项目目录。

找不到、无法读取、非 UTF-8 或空的 `SKILL.md` 会跳过；单文件最多 64 KiB，所有 skills 的原始文本合计最多 1 MiB，超预算的文件也跳过。默认静默，`-v` 显示加载和跳过原因；stdout 仍只输出最终回答。

指定 skill 的完整 `SKILL.md`（包含 frontmatter）会作为独立的 skills 段加入系统提示词，不预先加载引用资料或运行脚本。这是显式选择后的直接加载，不是先列出全部 skills 再由模型选择。提示词同时提供名称和实际目录；相对引用以该目录为基准，模型通过 shell 使用绝对路径按需读取。能力约束和用户明确任务优先于 skill 指令。上下文恢复保留整个系统提示词，因此已加载的 skills 不会被恢复移除；但 skills 总量计入不可约简部分，超出模型窗口时不会自动截断，只会出现在退出诊断里。

通过符号链接安装的 skill 目录会解析为实际目录；选中的实际目录额外授予只读访问，未选中的工作目录外的 skill 不获得此权限。读取文件的符号链接仍检查实际目标，拒绝逃出工作目录和选中 skill 目录的读取；`SKILL.md` 自身指向 skill 目录外时跳过。写权限仍只限启动 cwd 及其子目录，网络权限仍由 `--net` 控制；skill 的 `allowed-tools` 等 frontmatter 不改变权限，也不会注册新工具。外部引用可使用 `cat`、`head` 等现有只读命令；脚本执行继续受当前 Guard 约束，不保证其他 Agent 专用技能或复杂脚本兼容。

当前只支持名称输入，自定义单个 skill 目录或包含多个 skills 的目录留待后续扩展。

## 上下文过长恢复

正常运行只追加消息，不主动裁剪。只有上游明确报告上下文过长才触发恢复：HTTP 400/413/422 的有界 JSON 错误体，或 JSON/SSE 响应中的 `error`。识别 `context_length_exceeded` / `context_window_exceeded` 的 code/type，以及明确描述 context length/window exceeded 的消息；普通 400、鉴权失败、限流、服务器错误和超时不触发。

恢复是一架三级阶梯，按丢失的信息量从小到大依次尝试；上一级还能帮上忙时不会动用下一级。

1. **压缩旧轮次**。除最新一轮外的每个工具调用压缩成一行骨架：命令（截断显示）、exit code、stdout/stderr 字节数、超时与截断标记。全部事实来自已经解析的工具结果，不调用模型、不做 tokenizer、可重复执行。骨架写入索引 2 的单条摘要槽，**后续恢复重建而不是追加**，因此总长度有 64 KiB 上限，超出时丢弃最旧的条目。此级不删除任何轮次、不重跑任何命令。
2. **截断最新一轮的工具结果**。每个结果保留头 2 KiB + 尾 2 KiB，中间标注省略字节数。`exit_code`、`timed_out`、`truncated` 和 `tool_call_id` 配对全部保留；承载 `tool_calls` 的 assistant 消息不动，因此不会产生悬空调用。
3. **截断原始输入**。messages[1] 保留头 32 KiB + 尾 32 KiB。系统提示词与已加载的 skills 始终不被改写。

各级都会在历史里留下可识别的标记，阶梯据此判断该级是否还需要执行，因此重复的上下文错误不会把同一段内容反复裁剪，也不会把摘要槽越堆越多。

**损失边界（契约）**：第 1 级不丢命令与退出状态，只丢输出正文；第 2 级丢失的输出**不可恢复**，因为产生它的进程已经退出；第 3 级是**数据损毁**——管道输入只存在于 messages[1]，从未落盘，被省略的中间部分无法重新读取，只能重新执行产生它的命令。第 3 级会**无条件**向 stderr 打印警告（不需要 `--verbose`），摘要与说明也会要求模型在中间内容确实重要时于最终回答中说明。工作区文件不在此列：模型随时可以重新读取它们。

每次上下文失败执行一级恢复并重试模型一次；重试计入 `--max-steps`，不重放已处理的 shell。新的上下文错误从刚刚执行的那一级继续（该级仍然有效就再用一次），直至三级用尽。三级都无法再改变历史时，以错误退出，并报告系统提示词、原始输入、最新轮次、skills 与总字节数——即恢复无法触及的各个部分。`--verbose` 打印每一级的具体动作。

例如 `git diff | ma '总结这些改动'`：若 diff 超过模型窗口，先压缩此前的工具轮次（这里通常没有），再截断最新结果，最后截断 diff 本身并打印警告。此前的删除式恢复在这种形状下只能释放个位数百分比，甚至无可删轮次而直接失败。

当前不引入模型摘要、tokenizer、主动上下文预算或额外配置。压缩会丢失输出正文，截断不保证语义无损；它也可能降低缓存复用。

## 权限

权限在进程启动时授予，运行中不询问、不升级；拒绝作为工具结果反馈给模型。

| 启动方式 | 工作目录数据读取 | 写权限范围 | shell 网络 |
| --- | --- | --- | --- |
| 默认 | 允许 | 拒绝 | 拒绝 |
| `--write` | 允许 | 启动时 cwd 及其子目录 | 拒绝 |
| `--net` | 允许 | 拒绝 | 允许 |
| `--write --net` | 允许 | 启动时 cwd 及其子目录 | 允许 |

模型 API 请求始终允许，与 shell 的 `--net` 分开。系统可执行文件和运行所需的系统资源仍可使用。`--skills` 选中的 skill 目录额外允许只读访问；除此之外不支持额外授权目录。

同一份 Policy 生成模型指令并执行 Guard 检查。Guard 拦截常见写命令、文件重定向、明显网络命令；检查文字路径中的父目录、已有符号链接和常见输出选项。`/dev/null` 是可用输出目标，文件描述符重定向如 `>&2` 可用。

**Guard 是 best-effort safety gate，不是安全沙箱。** 未知 CLI、程序内部写入/网络、构建脚本、Git hooks、配置影响和检查后路径变化都可能产生未被识别的副作用；这些能力标志不能提供 OS 强制隔离。运行模型生成的命令时应使用信任的环境，强隔离留给后续 OS Sandbox。

只读文件命令（`cat head tail ls stat wc du file readlink`）支持工作目录内的通配符，例如 `cat *.rs`、`wc -l *.rs`、`ls ./*.rs`、`cat src/*/*.rs`。执行前同时检查原样路径（无匹配时 shell 使用它）与逐级匹配的候选，拒绝越界目录和符号链接；每个参数最多检查 16384 个目录项，超过时要求缩小范围。支持 `*`、`?` 和普通字符范围；字符范围与引号或转义混用时，会保守拒绝并提示改用文字路径或 `*` / `?`。引号内的通配符保持文字含义。Guard 按宿主 locale 匹配字符，与 `bash`、busybox `ash` 一致；Debian 和 Ubuntu 的 `/bin/sh` 是 `dash`，它按字节匹配 `?`，因此那里的 `?.rs` 不会匹配 UTF-8 文件名（如 `你.rs`），Guard 也会保守地不展开这些候选。

为保持 Guard 简单，V0 仍拒绝变量展开、命令替换、嵌套 shell、控制流、环境前缀、提权、`ln` 和 `xargs` / `find -exec` 等命令转发。`{}` 作为普通字面量接受；`find -exec cat {} +` 的拒绝会明确指出 `find -exec`，真正的 shell 分组仍拒绝。`cd` 不与 pipeline、后台分支或 `||` 混用；清除 `CDPATH`，防止环境改变实际工作目录。写路径必须是文字路径，不支持写目标通配符。下载写入使用显式输出路径；`curl -O` 等隐式文件名和无法检查的复合写选项会被拒绝。复杂命令可拆成多次 shell 调用，文本模式和脚本参数可使用单引号。包管理器和构建工具保守地视为写操作；支持的离线标志（例如 `cargo --offline`、`mvn -o`）可免除明显网络检查。其他复杂语法可能误拒绝，由模型调整命令。

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
- `API_KEY`（及宿主可能残留的旧 `MA_API_KEY`、`OPENAI_API_KEY`）不传给 shell 子进程；其他宿主环境仍继承。
- 普通 HTTP 错误不自动重试；只有上述明确上下文过长错误可按阶梯恢复并重试一次。不跟随重定向；TLS 验证使用依赖内置的根证书，不关闭证书校验。

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
