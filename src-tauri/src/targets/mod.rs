//! 部署目标 (Deployment Target): 项目 × 服务器 bindings (ticket 09).
//!
//! A deployment target binds a project to a server and carries the upload
//! directory plus the 启动命令组 / 停止命令组 (command groups). This ticket
//! delivers CRUD + persistence only — execution (upload / restart) belongs to
//! ticket 10.
//!
//! Referential integrity (documented decision): the bindings are dependent
//! rows, so deleting the referenced project or server **cascades** to deleting
//! the deployment targets (`ON DELETE CASCADE`, enforced via
//! `PRAGMA foreign_keys = ON` in `Store::open`). Blocking deletion was the
//! alternative, but a server a user no longer uses should not have to be
//! unbound from every project first.

pub mod commands;
pub mod repo;
