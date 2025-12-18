// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Utilities for Git.

use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;

use crate::core::release::errors::Result;

/// Force-create an ancestor-less branch containing a directory tree.
#[derive(Debug, Eq, PartialEq, Parser)]
pub struct RebootBranchCommand {
    #[structopt(
        long = "message",
        short = 'm',
        help = "The commit message",
        default_value = "Reboot branch"
    )]
    message: String,

    #[structopt(help = "The branch to reboot")]
    branch: String,

    #[structopt(help = "The root directory for the new tree")]
    root: PathBuf,
}

impl RebootBranchCommand {
    pub fn execute(self) -> Result<i32> {
        let repo = git2::Repository::open_from_env().context("couldn't open Git repository")?;
        let mut index = repo.index().context("couldn't open Git index")?;

        let sig = repo
            .signature()
            .or_else(|_| git2::Signature::now("belaf", "belaf@devnull"))?;

        let ref_name = format!("refs/heads/{}", &self.branch);

        repo.set_workdir(&self.root, false)
            .context("couldn't reset repo working directory")?;

        index.clear().context("couldn't clear index")?;
        index
            .add_all(["*"], git2::IndexAddOption::FORCE, None)
            .context("couldn't add new tree to index")?;
        let tree_id = index
            .write_tree()
            .context("couldn't write new index tree")?;
        let tree = repo
            .find_tree(tree_id)
            .context("couldn't recover new tree")?;

        let commit_id = repo
            .commit(None, &sig, &sig, &self.message, &tree, &[])
            .context("couldn't create new commit")?;

        repo.reference(&ref_name, commit_id, true, "reboot branch")?;

        Ok(0)
    }
}

#[derive(Debug, Eq, PartialEq, Parser)]
pub enum GitUtilCommands {
    #[structopt(name = "reboot-branch")]
    /// Force-create an ancestor-less branch
    RebootBranch(RebootBranchCommand),
}

#[derive(Debug, Eq, PartialEq, Parser)]
pub struct GitUtilCommand {
    #[command(subcommand)]
    command: GitUtilCommands,
}

impl GitUtilCommand {
    pub fn execute(self) -> Result<i32> {
        match self.command {
            GitUtilCommands::RebootBranch(o) => o.execute(),
        }
    }
}
