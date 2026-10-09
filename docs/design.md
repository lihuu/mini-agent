# Minimal Agent V0 设计

目的：一次性运行的 Rust executable `ma`，供人和程序通过参数、stdin、stdout、stderr 调用。永久坚持 One model. One tool. One loop.

## 已确认的边界

- 唯一模型协议为 OpenAI-compatible Chat Completions；默认同步 SSE 流式接收，不使用 SDK，兼容直接返回 JSON 的响应。
- 唯一工具 `shell(command)`。循环只负责请求模型、执行工具、追加工具结果、输出 final。
- macOS、Linux 为 V0 目标。Windows 暂不实现。无 Session、Memory、MCP、Planner、Provider 抽象、GUI、常驻后台工作。Skills 仅支持命令行显式选择后的指令加载。
- 同步 HTTP；少量普通函数，循环和权限逻辑在 `src/main.rs`，skill 加载与提示词拼装在 `src/skills.rs`。
- 默认 workspace read、write deny、shell network deny。模型 HTTP 请求不受 shell 网络开关影响。
- 用户已明确：`--write` 只授予启动时当前工作目录及其子目录的写权限。`--skills` 为选中的实际 skill 目录增加只读访问，不增加自定义写授权目录。
- 配置文件只提供连接设置（`base_url`、`api_key`、`model`），默认 `~/.config/ma/config.json`，可用 `MA_CONFIG` 覆盖；权限永远不能来自配置文件。
- 权限在启动时确定，运行中绝不询问或升级权限。同一 Policy 产生模型指令并驱动 Guard。
- Guard 拦截明显违规，包含常见写命令和重定向的路径检查；使用 canonical path 检查已存在的路径与符号链接。未知 executable、程序内部副作用及 shell 动态计算不能被可靠静态判断，因此不是 OS Sandbox。
- 只读文件命令可使用工作目录通配符，执行前按路径组件匹配并检查目录与符号链接边界，扫描预算每个参数 16384 个目录项。保留引号含义；混合引号的字符范围保守拒绝。写目标仍只接受文字路径。

## 运行契约

- `--base-url`/`BASE_URL`、`--api-key`/`API_KEY`、`--model`/`MODEL`；模型设置只读取这三个环境变量，无命名前缀或旧名称后备。配置文件以更低的优先级提供同样的三个键。
- `--skills name1,name2` 只按目录名查找启动 cwd 的 `.agents/skills/` 和 `$HOME/.agents/skills/`；项目优先，读取失败回退，仍失败则跳过。不传就不加载。完整 `SKILL.md` 和实际目录进入系统提示词的 skills 段，保持两条初始消息（system、user）及现有裁剪契约。资源按需由 shell 读取，不预先运行脚本、不解析权限 frontmatter、不注册新工具。单文件 64 KiB、原始指令总量 1 MiB；加载和跳过仅在 verbose 记录。自定义目录输入尚未支持。
- Base URL 是 API 根目录，例如 `https://example.com/v1`，客户端追加 `/chat/completions`。
- Prompt 来自位置参数；非终端 stdin 作为补充文本，也支持仅 stdin。限制输入为 1 MiB；shell stdin 固定关闭，避免交互等待。
- help / version 可出现在 Prompt 后，在配置和输入读取前处理；跳过其他选项的值，并以 `--` 为 Prompt 分界。
- stdout 只输出完整 final；stderr 输出本地错误。`--verbose` / `-v` 实时追加模型文本、轮次、shell 命令、工具输出及结果。默认关闭过程日志，不引入日志框架。成功退出 0，参数/输入错误 2，HTTP/协议错误 1，耗尽步数 3。
- `max_steps` 默认 200，指模型请求次数。最后一步不再执行无法反馈给模型的工具调用。
- 请求 `stream: true`；SSE 按事件而非传输 chunk 解析，支持 UTF-8、开头 BOM、LF/CRLF/CR、心跳及 usage-only chunk。content、reasoning_content、refusal 和按 index 区分的工具参数分别累积；额外字段保留并递归合并对象，冲突值按协议错误退出。`finish_reason` 和 `[DONE]` 必须均出现才认可完整响应。完整接收、校验后执行工具；断流不输出 partial final，也不执行部分工具。整个响应仍限 4 MiB，HTTP timeout 覆盖流式读取。verbose 输出为 best-effort，管道写满时不等待，以免影响请求读取和 shell 超时。
- HTTP timeout 默认 120 秒，shell timeout 默认 30 秒。可用启动参数调整，必须大于零。
- shell stdout/stderr 分别最多保留 64 KiB，超出后继续排空管道并标记截断；无长期后台读取线程。
- Unix shell 使用 `/bin/sh -c`，固定 cwd 为启动目录；独立进程组允许超时后清理后代。不把模型 API key 环境变量传给 shell。
- 清除 shell 的 `CDPATH`，`cd` 仅支持简单顺序执行；拒绝和管道、后台、`||` 混合的 `cd`。拒绝 `ln` 和无法确定写目标的下载形式，避免明显的路径检查空窗。仍不模拟完整 shell 和 executable 的内部状态。
- SIGINT / SIGTERM / SIGHUP 中断时清理当前工具进程组，按 `128 + signal` 退出；SIGKILL 和脱离进程组的进程不作强保证。
- 工具结果采用 JSON 字符串，包含 stdout、stderr、exit_code、timed_out、truncated；拒绝和执行错误也作为工具结果反馈模型。
- 多工具调用按返回顺序执行并逐条反馈。assistant 消息（含 reasoning_content 等兼容字段）保留；校验工具 ID、工具名、参数和最终 finish_reason，防止错误结束或执行未知工具。
- 普通 HTTP 错误不重试；只有明确上下文过长错误触发按完整轮次裁剪旧历史，保留 system、原始 user 和最新工具轮次。尽量移除约一半历史字节，插入可替换的裁剪提示，然后重试模型一次；不重放 shell，重试计入 max_steps。无可裁剪历史或重试仍失败则退出。不做主动裁剪、模型摘要、tokenizer 或新配置。

## 验收

端到端本地 mock HTTP 测试验证 wire protocol、stdin、多轮/多工具、权限拒绝、步骤上限、错误和超时，以及 SSE 传输分片、交错工具参数、断流、总字节预算和上游未结束前可见的 verbose 文本；真实 shell 测试验证实时输出、超时、截断和目录边界。运行 fmt、clippy、cargo test、release build 并测量体积：目标 <5 MB，硬限制 <10 MB。macOS 本机验收，Linux 用 CI 或一次性容器验收；没有实际运行的跨平台或私人 endpoint 测试不得声称通过。
