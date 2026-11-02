//! 服务器 (Server): SSH connection management (ticket 04).
//!
//! A Server only carries connection information (host, port, user,
//! credentials). Where artifacts go and how services restart belongs to the
//! 部署目标 (Deployment Target), not here (see GLOSSARY.md).

pub mod commands;
pub mod repo;
pub mod ssh;
pub mod ssh;
