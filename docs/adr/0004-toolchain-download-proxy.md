# 工具链下载代理策略：显式配置 > 环境变量 > 直连

托管工具链下载（github.com releases）在部分网络环境直连不通，而下载层原本是纯直连设计（reqwest 未启用系统代理特性，GUI 启动也不继承 shell 的代理环境变量）。票 27 决定：代理优先级为设置页显式配置 > 环境变量 > 直连；**配置了代理（显式或 env）就交给代理处理连接，失败不回退直连**；未配置代理时保留逐地址直连回退（规避 hyper 不做 happy-eyeballs）。

## Considered Options

- **读取系统代理**（reqwest `macos-system-configuration` 特性）：拒绝——行为不可预测（TUN 模式下系统代理为空、部分工具只改终端 env），诊断困难；显式配置已覆盖 GUI 启动场景。
- **代理失败静默回退直连**：拒绝——用户显式配代理通常说明直连不通，静默回退会让"为什么连得上/连不上"难以诊断，且可能违背网络环境本意。
- **SOCKS5 支持**：暂缓——本地代理工具（Clash/V2Ray 等）都提供 HTTP 端口，需要时再开 reqwest `socks` 特性。

## Consequences

- 显式代理走 `no_proxy() + proxy(url)` 的专用 client，不做本地 DNS 解析与逐地址回退（连接目标由代理决定）。
- 代理只作用于托管工具链下载（`toolchain/download.rs`）；Maven 依赖下载走 mvnd/Java，属独立机制，需要时另开一票。
