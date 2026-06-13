use clap::Args;
use std::path::Path;

use crate::extensions::ErrorReportExt;
use knot_client::KnotClient;
use knot_terminal::{ErrorReport, TaskEngine};

#[derive(Args)]
pub struct PgArgs {
    #[arg(short, long)]
    pub detach: bool,
}

pub async fn execute(
    args: PgArgs,
    target_dir: &Path,
    engine: &TaskEngine<'static>,
) -> anyhow::Result<()> {
    let mut boot_task = engine
        .task("Connecting to Knot environment")
        .with_group(Some("Прекольчики"))
        .with_stage("1", "Достаем печеньки...", true)
        .with_stage("2", "А это нам не надо", true)
        .with_stage("3", "Достаем кака колу...", true)
        .with_group(None)
        .with_stage("4", "Connecting to workspace...", false)
        .with_stage("5", "Connecting to IPC socket...", false)
        .start(true);

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    let mut test_task = engine
        .sequence("Test")
        .with_stage("stage 1")
        .with_group(Some("Test"))
        .with_stage("stage 2")
        .start(false);

    let fail_task = engine.task("Fail Task").start(false);
    let step = engine.step("Test Step", true);

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    boot_task.ok_by_id("1", "печеньки на столе");
    test_task.ok("stage 1");

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    boot_task.ok_by_id("3", "кака кола на столе");
    test_task.skip("stage 2");
    fail_task.fail(ErrorReport::new("Task failed").with_solution("no solution"));

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    boot_task.skip_by_id("2", "а ведь действительно");
    step.ok("Success");

    boot_task.run_by_id("4", Some("booting client..."));
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let client_res = KnotClient::connect_or_launch(&target_dir).await;
    let _client = match client_res {
        Ok(c) => {
            anyhow::bail!("connected");
        }
        Err(e) => {
            boot_task.ok_by_id("inserted", "gutt");
            let report = ErrorReport::from_error(e);
            boot_task.fail_by_id("4", report);
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            for i in 0..10 {
                println!("blabla {}", i);
            }
            anyhow::bail!("Aborting due to connection error");
        }
    };

    Ok(())
}
