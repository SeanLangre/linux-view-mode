//! HDMI-CEC power control for the TV. Uses `cec-client` (libcec, e.g. a Pulse-Eight adapter) if
//! installed, else `cec-ctl` (v4l-utils, for the kernel's /dev/cec* devices). The device is
//! `COUCHTUBE_CEC_DEVICE` (default `/dev/cec0`) and only matters for `cec-ctl`.
use anyhow::{Context, Result, bail};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, PartialEq)]
enum Tool {
    Client,
    Ctl(String),
}

fn device() -> String {
    std::env::var("COUCHTUBE_CEC_DEVICE").unwrap_or_else(|_| "/dev/cec0".into())
}

async fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok()
}

async fn tool() -> Option<Tool> {
    if installed("cec-client").await {
        return Some(Tool::Client);
    }
    let device = device();
    (installed("cec-ctl").await && std::path::Path::new(&device).exists()).then_some(Tool::Ctl(device))
}

/// Whether a CEC tool and (for the kernel interface) a device exist.
pub async fn available() -> bool {
    tool().await.is_some()
}

/// Arguments for `cec-ctl`: register as a playback device, then address the TV (logical address 0).
fn ctl_args(device: &str, on: bool) -> Vec<String> {
    let mut args = vec![format!("--device={device}"), "--playback".into(), "--skip-info".into(), "--to".into(), "0".into()];
    args.push(if on { "--image-view-on" } else { "--standby" }.into());
    args
}

pub async fn power(on: bool) -> Result<()> {
    let Some(tool) = tool().await else {
        bail!("no HDMI-CEC tool (cec-client or cec-ctl with /dev/cec0) found");
    };
    let mut command = match &tool {
        Tool::Client => {
            let mut command = Command::new("cec-client");
            // -s: one command from stdin then exit; -d 1: only errors.
            command.args(["-s", "-d", "1"]).stdin(Stdio::piped());
            command
        }
        Tool::Ctl(device) => {
            let mut command = Command::new("cec-ctl");
            command.args(ctl_args(device, on)).stdin(Stdio::null());
            command
        }
    };
    command.stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    let mut child = command.spawn().context("could not start the HDMI-CEC tool")?;
    if tool == Tool::Client {
        let line = if on { "on 0\n" } else { "standby 0\n" };
        child.stdin.take().context("no stdin")?.write_all(line.as_bytes()).await?;
    }
    let status = tokio::time::timeout(TIMEOUT, child.wait())
        .await
        .context("the HDMI-CEC tool timed out")??;
    if !status.success() {
        bail!("the HDMI-CEC tool failed ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ctl_args;

    #[test]
    fn addresses_the_tv() {
        assert_eq!(ctl_args("/dev/cec1", false).last().unwrap(), "--standby");
        assert_eq!(ctl_args("/dev/cec1", true).last().unwrap(), "--image-view-on");
        assert!(ctl_args("/dev/cec1", true).contains(&"--device=/dev/cec1".to_string()));
    }
}
