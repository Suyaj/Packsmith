//! 部署记录（ticket 12）：每次部署（手动 [`crate::deploy::commands::start_deploy`]
//! 与构建成功后的自动连锁部署）结束后，把每个部署目标的结果落一行。
//!
//! 首页服务器卡片显示"最近部署时间"：ticket 10 的部署执行本身不持久化任何
//! 记录，这里选择最简单且真实的方案——按部署结果记行，[`last_successful_per_server`]
//! 把"最近部署时间"定义为该服务器所有部署目标里最近一次**成功**部署的时间
//! （失败部署也落库但不算数，方便后续排查）。记录随部署目标级联删除。

use serde::Serialize;

use super::{Store, StoreError};

/// 一台服务器的最近成功部署时间（从未部署过为 `None`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerLastDeploy {
    pub server_id: i64,
    pub last_deploy_at: Option<String>,
}

/// 落一行部署结果（`operation` = 本次操作类型，票 23）。目标已不存在
/// （并发删除）时容忍并忽略。
pub fn record(
    store: &Store,
    target_id: i64,
    success: bool,
    operation: crate::deploy::engine::DeployOperation,
) -> Result<(), StoreError> {
    store.with(|conn| {
        conn.execute(
            "INSERT INTO deploy_records (target_id, success, operation)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM deployment_targets WHERE id = ?1)",
            rusqlite::params![target_id, success as i64, operation.as_str()],
        )?;
        Ok(())
    })
}

/// 落盘一次部署的每个目标结果（[`crate::deploy::engine::DeploySummary`]），
/// 统一记为本次的操作类型 `operation`（票 23）。
/// 失败目标不阻塞成功目标的记录。
pub fn record_outcomes(
    store: &Store,
    summary: &crate::deploy::engine::DeploySummary,
    operation: crate::deploy::engine::DeployOperation,
) -> Result<(), StoreError> {
    for outcome in &summary.targets {
        let success = outcome.status == crate::deploy::engine::TargetStatus::Success;
        record(store, outcome.target_id, success, operation)?;
    }
    Ok(())
}

/// 每台已知服务器一行：该服务器所有部署目标里最近一次成功部署的时间。
pub fn last_successful_per_server(store: &Store) -> Result<Vec<ServerLastDeploy>, StoreError> {
    store.with(|conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, MAX(d.created_at)
             FROM servers s
             LEFT JOIN deployment_targets t ON t.server_id = s.id
             LEFT JOIN deploy_records d ON d.target_id = t.id AND d.success = 1
             GROUP BY s.id
             ORDER BY s.id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ServerLastDeploy {
                    server_id: row.get(0)?,
                    last_deploy_at: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}
