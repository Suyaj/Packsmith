//! 部署执行 (Deployment execution, ticket 10): upload + command-group
//! restart, orchestrated by [`engine::run_deploy`] over the
//! [`engine::DeployTransport`] seam.
//!
//! Phase order per 部署目标 (deployment target): SFTP upload (original
//! filename, overwrite) → 停止命令组 (stop group) → on success 启动命令组
//! (start group). A failed stop group aborts the restart: the start group
//! never runs, the stop output is surfaced, and the user may later run the
//! start group alone (单独执行启动命令组). Targets deploy sequentially.

pub mod commands;
pub mod engine;
pub mod transport;
