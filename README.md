# Minimal Agent

**One model. One tool. One loop.**

A tiny, one-shot agent in a single binary. Give it a task; it calls a model, runs shell commands, prints the answer, and exits.

`ma` 是一个用 Rust 写的小型命令行 Agent：一个模型、一个 `shell` 工具、一个执行循环。它使用宿主已有的 CLI 完成任务，给出最终回答后退出。没有会话管理、插件或后台服务。

- 单二进制，release 约 1.3–1.5 MB，支持 macOS / Linux。
- 使用支持工具调用的 OpenAI-compatible Chat Completions，默认流式接收。
- 支持管道输入，使用 `-v` 实时查看执行过程。

## 快速开始

从源码构建需要 Rust 1.85+ 和 C 编译器；Agent 本身运行无需安装 Rust、Python、Node 或 JVM，工具命令使用系统 `/bin/sh`。

```sh
git clone https://github.com/lihuu/mini-agent.git
cd mini-agent
cargo build --release --locked

export BASE_URL='https://your-endpoint.example/v1'
export MODEL='your-model'
export API_KEY='your-key'

./target/release/ma -v '分析当前项目'
```

`BASE_URL` 填 API 根地址，程序会追加 `/chat/completions`；默认是 `https://api.openai.com/v1`。模型和密钥必须提供，只读取这三个环境变量。

## 用法

```sh
# 分析当前目录
./target/release/ma '解释这个项目的结构'

# 从 stdin 接收补充上下文
git diff | ./target/release/ma '总结这些修改'

# 允许修改当前目录及其子目录，并实时显示执行过程
./target/release/ma -v --write '修复配置文件'

# 同时允许 shell 联网
./target/release/ma -v --write --net '升级项目依赖'

# 保存最终回答，执行过程仍显示在终端
./target/release/ma -v '分析构建失败原因' > answer.txt
```

默认只在 stdout 输出完整最终回答；`-v` / `--verbose` 开启后，模型文本、shell 命令和输出、耗时与结果实时写入 stderr。

仅在上游明确报告上下文过长时，删除最旧的一批完整执行轮次并重试模型一次。保留原始任务与最近轮次，不自动生成摘要，也不重复执行已处理的 shell 命令。`-v` 会显示裁剪过程。

## 常用参数

| 参数 | 说明 | 默认值 |
| --- | --- | --- |
| `--base-url` | 模型 API 根地址 | `https://api.openai.com/v1` |
| `--model` | 模型名称 | `MODEL` |
| `--api-key` | API 密钥，建议用环境变量传入 | `API_KEY` |
| `--write` | 允许写入启动时的当前目录及其子目录 | 关闭 |
| `--net` | 允许 shell 联网 | 关闭 |
| `-v, --verbose` | 实时显示执行过程 | 关闭 |
| `--max-steps` | 最多请求模型的次数 | `200` |
| `--http-timeout` | 单次模型请求超时，秒 | `120` |
| `--shell-timeout` | 单次 shell 命令超时，秒 | `30` |

完整帮助：`./target/release/ma --help`。参数优先于环境变量。

## 权限边界

默认检查并拒绝常见写操作和 shell 网络命令；模型 API 请求始终允许。权限在启动时确定，运行中不弹出审批。

**内置 Guard 是 best-effort 检查，不是安全沙箱。** 未知 CLI、构建脚本等仍可能产生检查未识别的副作用。需要强隔离时，应使用容器或虚拟机。详见 [SECURITY.md](SECURITY.md)。

参数细节、命令限制、输出契约和测试方法见 [使用说明](docs/usage.md)；设计与验收记录见 [设计说明](docs/design.md)。

## License

[MIT](LICENSE)。
