//! 数据库迁移测试。
//!
//! 这些测试用于验证 `TASK-004` 的 `SQLite` 建连、迁移与启动自检能力。

use dshns_agent::infra::db::{DatabaseTarget, SqliteDatabase};

#[test]
fn 初始化数据库时应创建文档约定的核心表() {
    let database = SqliteDatabase::open(DatabaseTarget::InMemory).expect("内存数据库打开失败");

    let report = database.initialize().expect("数据库初始化失败");

    assert!(report.self_check_passed);
    assert_eq!(report.migrated_table_names.len(), 9);
    assert!(
        report
            .migrated_table_names
            .contains(&"workspaces".to_string())
    );
    assert!(database.table_exists("sessions").expect("查询表存在性失败"));
    assert!(
        database
            .table_exists("event_logs")
            .expect("查询表存在性失败")
    );
}

#[test]
fn 重复执行迁移应保持幂等() {
    let database = SqliteDatabase::open(DatabaseTarget::InMemory).expect("内存数据库打开失败");

    database.initialize().expect("首次初始化失败");
    let second_report = database.initialize().expect("重复初始化失败");

    assert!(second_report.self_check_passed);
    assert_eq!(second_report.migrated_table_names.len(), 9);
}
