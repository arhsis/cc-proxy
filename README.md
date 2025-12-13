# cc-proxy

Language | 语言: [English](#english) · [中文](#中文)

## English

**A lightweight, intelligent HTTP proxy for Claude Code and Codex CLIs.**

`cc-proxy` sits between your AI CLI tools and upstream providers. It optimizes for cost and reliability by enforcing **sticky routing** (to maximize prompt caching) and handling **automatic failover** seamlessly.

## ⚡ Key Features

  * **💰 Sticky Routing**: Maintains provider affinity for 5 minutes. This keeps the prompt cache warm, potentially reducing API costs.
  * **🛡️ Automatic Failover**: If a provider goes down, `cc-proxy` instantly retries the request with the next provider in your priority list.
  * **⚙️ Auto-Configuration**: Automatically manages the proxy settings for Claude Code and Codex CLIs—no manual export needed.
  * **🚀 Lightweight**: A single Rust binary with no database or heavy dependencies.

-----

## 🛠️ Installation

### Prerequisites

  * **Rust**: Ensure you have `cargo` installed.

### Build from Source

```bash
# Clone the repository
git clone https://github.com/arhsis/cc-proxy.git
cd cc-proxy

# Build release binary
cargo build --release

# Install globally
sudo cp target/release/cc-proxy /usr/local/bin/
```

-----

## 🚀 Usage

### Basic Commands

```bash
# Start the proxy (daemon mode)
# This automatically configures Claude & Codex to use the proxy.
cc-proxy start

# Check connection status and current routing
cc-proxy status

# Stop the proxy and revert CLI configurations
cc-proxy stop
```

### Configuration

Create your configuration file at `~/.cc-proxy/provider.json`.

You can define separate provider lists for **Codex** and **Claude**. The proxy tries providers in the order listed (top down).

**Example `provider.json`**:

```json
{
  "providers": {
    "codex": [
      { "apiUrl": "https://api.openai.com/v1", "apiKey": "YOUR_OPENAI_API_KEY" },
      { "apiUrl": "https://api.openai.com/v1", "apiKey": "YOUR_OPENAI_API_KEY_1" }
    ],
    "claude": [
      { "apiUrl": "https://api.anthropic.com", "apiKey": "YOUR_ANTHROPIC_API_KEY" }
    ]
  }
}
```

-----

## 中文

**为 Claude Code 与 Codex CLI 提供的轻量智能 HTTP 代理。**

`cc-proxy` 位于本地 CLI 与上游模型服务之间，通过 **粘性路由**（维持 5 分钟的同源请求以利用缓存）和 **自动故障切换**，在可靠性与成本间取得平衡。

### ⚡ 核心特性

  * **💰 粘性路由**：保持同一提供商 5 分钟，利用提示缓存降低调用成本。
  * **🛡️ 自动故障切换**：上游不可用时自动切到下一个提供商。
  * **⚙️ 自动配置**：无需手动导出代理变量，自动配置 Claude Code 与 Codex CLI。
  * **🚀 轻量单可执行文件**：纯 Rust 实现，无数据库与重依赖。

-----

### 🛠️ 安装

#### 先决条件

  * **Rust**：需要已安装 `cargo`。

#### 源码构建

```bash
# 克隆仓库
git clone https://github.com/yourusername/cc-proxy.git
cd cc-proxy

# 构建发布版本
cargo build --release

# 全局安装
sudo cp target/release/cc-proxy /usr/local/bin/
```

-----

### 🚀 使用

#### 基本命令

```bash
# 启动代理（守护模式），自动配置 Claude & Codex 代理
cc-proxy start

# 查看连接状态与当前路由
cc-proxy status

# 停止代理并恢复 CLI 配置
cc-proxy stop
```

#### 配置

在 `~/.cc-proxy/provider.json` 创建配置文件，为 **Codex** 与 **Claude** 分别设置提供商列表（按顺序优先）。

**示例 `provider.json`**：

```json
{
  "providers": {
    "codex": [
      { "apiUrl": "https://api.openai.com/v1", "apiKey": "YOUR_OPENAI_API_KEY" },
      { "apiUrl": "https://api.openai.com/v1", "apiKey": "YOUR_OPENAI_API_KEY_1" }
    ],
    "claude": [
      { "apiUrl": "https://api.anthropic.com", "apiKey": "YOUR_ANTHROPIC_API_KEY" }
    ]
  }
}
```

-----

## License

MIT