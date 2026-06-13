use clap::Args;
use std::path::Path;

use knot_terminal::TaskEngine;

#[derive(Args)]
pub struct UpArgs {
    #[arg(short, long)]
    pub detach: bool,
}

pub async fn execute(
    args: UpArgs,
    target_dir: &Path,
    engine: &TaskEngine<'static>,
) -> anyhow::Result<()> {
    let task = engine.task("Test Task").start(false);
    task.fail("fail");
    Ok(())
}
