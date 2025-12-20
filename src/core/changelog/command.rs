use std::io::Write;
use std::process::{Command, Stdio};
use std::{env, str, thread};

use super::error::{Error, Result};

pub fn run(command: &str, input: Option<String>, envs: Vec<(&str, &str)>) -> Result<String> {
    log::trace!("Running command: {:?}", command);
    let mut child = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .envs(envs)
            .args(["/C", command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .current_dir(env::current_dir()?)
            .spawn()
    } else {
        Command::new("sh")
            .envs(envs)
            .args(["-c", command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .current_dir(env::current_dir()?)
            .spawn()
    }?;

    if let Some(input) = input {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::CommandError("stdin is not captured".to_string()))?;
        thread::spawn(move || {
            stdin
                .write_all(input.as_bytes())
                .expect("Failed to write to stdin");
        });
    }

    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(str::from_utf8(&output.stdout)?.to_string())
    } else {
        for output in [&output.stdout, &output.stderr] {
            let output = str::from_utf8(output)?.to_string();
            if !output.is_empty() {
                log::error!("{}", output);
            }
        }
        Err(Error::CommandError(format!(
            "command exited with {:?}",
            output.status
        )))
    }
}
