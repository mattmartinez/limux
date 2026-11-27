//! CLI side of the workspace card: `set-status`, `clear-status`,
//! `list-status`, `set-description`, the agent lifecycle rows the hooks
//! write, and `refresh-pr`.
//!
//! The flags follow cmux (`cmux set-status <key> <value> --icon --color
//! --priority`) so hook scripts written for cmux work with `limux` too.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

/// Flags of the card commands that take a value.
const VALUE_FLAGS: [&str; 6] = [
    "--workspace",
    "--icon",
    "--color",
    "--priority",
    "--cwd",
    "--timeout",
];

/// Positional arguments, skipping every flag and the value of a flag that takes one.
fn positionals(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if VALUE_FLAGS.contains(&arg.as_str()) {
            iter.next();
        } else if !arg.starts_with("--") {
            out.push(arg.clone());
        }
    }
    out
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

/// `--workspace`, else `LIMUX_WORKSPACE_ID`, so an agent inside Limux targets its own card.
fn workspace_param(
    params: &mut Map<String, Value>,
    args: &[String],
    env_value: &impl Fn(&str) -> Option<String>,
) {
    if let Some(workspace) = flag_value(args, "--workspace")
        .or_else(|| env_value("LIMUX_WORKSPACE_ID"))
        .filter(|value| !value.trim().is_empty())
    {
        params.insert("workspace_id".into(), Value::String(workspace));
    }
}

/// `set-status <key> <value...> [--icon <name>] [--color <#hex|name>] [--priority <n>]`
pub fn build_set_status_params(
    args: &[String],
    env_value: impl Fn(&str) -> Option<String>,
) -> Result<Value> {
    let words = positionals(args);
    let (key, value) = match words.split_first() {
        Some((key, rest)) if !rest.is_empty() => (key.clone(), rest.join(" ")),
        _ => bail!("set-status needs a key and a value: limux set-status <key> <value>"),
    };
    let mut params = Map::new();
    params.insert("key".into(), Value::String(key));
    params.insert("value".into(), Value::String(value));
    if let Some(icon) = flag_value(args, "--icon") {
        params.insert("icon".into(), Value::String(icon));
    }
    if let Some(color) = flag_value(args, "--color") {
        params.insert("color".into(), Value::String(color));
    }
    if let Some(priority) = flag_value(args, "--priority") {
        let priority: i64 = priority
            .trim()
            .parse()
            .map_err(|_| anyhow!("--priority must be an integer"))?;
        params.insert("priority".into(), json!(priority));
    }
    workspace_param(&mut params, args, &env_value);
    Ok(Value::Object(params))
}

/// `clear-status [<key>]`: one row, or every row with no key.
pub fn build_clear_status_params(
    args: &[String],
    env_value: impl Fn(&str) -> Option<String>,
) -> Result<Value> {
    let mut params = Map::new();
    if let Some(key) = positionals(args).first() {
        params.insert("key".into(), Value::String(key.clone()));
    }
    workspace_param(&mut params, args, &env_value);
    Ok(Value::Object(params))
}

/// `list-status`
pub fn build_list_status_params(
    args: &[String],
    env_value: impl Fn(&str) -> Option<String>,
) -> Result<Value> {
    let mut params = Map::new();
    workspace_param(&mut params, args, &env_value);
    Ok(Value::Object(params))
}

/// `set-description <text...>`, or `set-description --clear`.
pub fn build_set_description_params(
    args: &[String],
    env_value: impl Fn(&str) -> Option<String>,
) -> Result<Value> {
    let text = positionals(args).join(" ");
    let clear = args.iter().any(|arg| arg == "--clear");
    if text.is_empty() && !clear {
        bail!("set-description needs text, or --clear to remove it");
    }
    let mut params = Map::new();
    params.insert(
        "description".into(),
        Value::String(if clear { String::new() } else { text }),
    );
    workspace_param(&mut params, args, &env_value);
    Ok(Value::Object(params))
}

/// Text output for the card commands when `--json` is not given.
pub fn describe_result(command: &str, payload: &Value) -> String {
    match command {
        "list-status" => {
            let rows: Vec<String> = payload["statuses"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            format!(
                                "{}\t{}",
                                row["key"].as_str().unwrap_or(""),
                                row["value"].as_str().unwrap_or("")
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut lines = Vec::new();
            if let Some(description) = payload["description"].as_str() {
                lines.push(format!("description\t{description}"));
            }
            lines.extend(rows);
            if lines.is_empty() {
                "no status rows".to_string()
            } else {
                lines.join("\n")
            }
        }
        "clear-status" => format!(
            "OK, removed {}",
            payload["removed"].as_u64().unwrap_or_default()
        ),
        _ => "OK".to_string(),
    }
}

/// What an agent hook event does to the agent's own status row, cmux style.
#[derive(Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// Set the row to this value, icon, colour and priority.
    Set(&'static str, &'static str, &'static str, i64),
    /// Remove the row when activity ends.
    Clear,
    /// Leave the row as it is.
    Keep,
}

/// Running on a new prompt, Needs input on a notification, and no row once
/// a turn finishes or the session ends.
pub fn lifecycle(event: &str) -> Lifecycle {
    match event {
        "UserPromptSubmit" | "prompt-submit" | "BeforeAgent" | "PreToolUse" => {
            Lifecycle::Set("Running", "bolt.fill", "#5c9cf5", 2)
        }
        "Notification" | "notification" | "PermissionRequest" => {
            // Match the OSC status priority for a request that needs action.
            Lifecycle::Set("Needs input", "bell.fill", "#e2c05c", 4)
        }
        "Stop" | "stop" | "finished" | "AfterAgent" | "SessionStart" | "session-start"
        | "SessionEnd" | "session-end" | "Cleanup" | "cleanup" => Lifecycle::Clear,
        _ => Lifecycle::Keep,
    }
}

/// The request that applies `event` to the row `agent_key`, if any.
pub fn lifecycle_params(agent_key: &str, event: &str, workspace: &str) -> Option<(String, Value)> {
    match lifecycle(event) {
        Lifecycle::Set(value, icon, color, priority) => Some((
            "workspace.set_status".to_string(),
            json!({
                "workspace_id": workspace,
                "key": agent_key,
                "value": value,
                "icon": icon,
                "color": color,
                "agent_activity": true,
                "priority": priority,
            }),
        )),
        Lifecycle::Clear => Some((
            "workspace.clear_status".to_string(),
            json!({ "workspace_id": workspace, "key": agent_key }),
        )),
        Lifecycle::Keep => None,
    }
}

/// Events after which the PR row is looked up again.
pub fn refreshes_pr(event: &str) -> bool {
    matches!(
        event,
        "Stop" | "stop" | "finished" | "AfterAgent" | "SessionStart" | "session-start"
    )
}

/// Start `limux refresh-pr` for this workspace in the background and return
/// at once, so an agent's hook never waits for GitHub.
pub fn spawn_pr_refresh(workspace: &str, cwd: Option<&str>) {
    use std::os::unix::process::CommandExt;
    if env_flag_disabled("LIMUX_PR_STATUS_DISABLED") {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut command = std::process::Command::new(exe);
    command.args(["refresh-pr", "--workspace", workspace]);
    if let Some(cwd) = cwd {
        command.args(["--cwd", cwd]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let _ = command.spawn();
}

fn env_flag_disabled(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

/// What `gh pr view` found for the branch checked out in a directory.
#[derive(Debug, PartialEq, Eq)]
pub enum PrLookup {
    Found { number: u64, state: String },
    NoPr,
    Unavailable,
}

/// Ask `gh` about the pull request for the branch in `cwd`, with a timeout.
pub async fn lookup_pr(cwd: &Path, timeout: Duration) -> PrLookup {
    let child = tokio::process::Command::new("gh")
        .args(["pr", "view", "--json", "number,state"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let Ok(child) = child else {
        return PrLookup::Unavailable;
    };
    let Ok(Ok(output)) = tokio::time::timeout(timeout, child.wait_with_output()).await else {
        return PrLookup::Unavailable;
    };
    parse_gh_pr_view(output.status.success(), &output.stdout, &output.stderr)
}

/// Interpret `gh pr view --json number,state`. gh exits non-zero with
/// "no pull requests found" when the branch has none.
pub fn parse_gh_pr_view(success: bool, stdout: &[u8], stderr: &[u8]) -> PrLookup {
    if !success {
        let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        return if stderr.contains("no pull requests found") {
            PrLookup::NoPr
        } else {
            PrLookup::Unavailable
        };
    }
    let Ok(value) = serde_json::from_slice::<Value>(stdout) else {
        return PrLookup::Unavailable;
    };
    match (value["number"].as_u64(), value["state"].as_str()) {
        (Some(number), Some(state)) => PrLookup::Found {
            number,
            state: state.to_ascii_lowercase(),
        },
        _ => PrLookup::Unavailable,
    }
}

/// The `pr` row for a lookup: set it, clear it, or leave it as it is when gh
/// could not answer.
pub fn pr_status_params(lookup: &PrLookup, workspace: &str) -> Option<(String, Value)> {
    match lookup {
        PrLookup::Found { number, state } => {
            let color = match state.as_str() {
                "open" => "#57c785",
                "merged" => "#b889f4",
                _ => "#8d8d8d",
            };
            Some((
                "workspace.set_status".to_string(),
                json!({
                    "workspace_id": workspace,
                    "key": "pr",
                    "value": format!("PR #{number} {state}"),
                    "icon": "arrow.triangle.pull",
                    "color": color,
                    "priority": -10,
                }),
            ))
        }
        PrLookup::NoPr => Some((
            "workspace.clear_status".to_string(),
            json!({ "workspace_id": workspace, "key": "pr" }),
        )),
        PrLookup::Unavailable => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn set_status_takes_cmux_flags_and_joins_the_value() {
        let params = build_set_status_params(
            &args(&[
                "qwen",
                "Needs",
                "input",
                "--icon",
                "bell.fill",
                "--color",
                "#e2c05c",
                "--priority",
                "3",
            ]),
            no_env,
        )
        .unwrap();
        assert_eq!(
            params,
            json!({"key": "qwen", "value": "Needs input", "icon": "bell.fill", "color": "#e2c05c", "priority": 3})
        );
    }

    #[test]
    fn set_status_targets_the_callers_workspace() {
        let params = build_set_status_params(&args(&["k", "v"]), |name| {
            (name == "LIMUX_WORKSPACE_ID").then(|| "ws-7".to_string())
        })
        .unwrap();
        assert_eq!(params["workspace_id"], "ws-7");
        let explicit = build_set_status_params(&args(&["k", "v", "--workspace", "ws-1"]), |_| {
            Some("ws-7".into())
        })
        .unwrap();
        assert_eq!(explicit["workspace_id"], "ws-1");
    }

    #[test]
    fn set_status_needs_key_and_value() {
        assert!(build_set_status_params(&args(&["only-key"]), no_env).is_err());
        assert!(build_set_status_params(&args(&["k", "v", "--priority", "x"]), no_env).is_err());
    }

    #[test]
    fn clear_and_description() {
        assert_eq!(
            build_clear_status_params(&args(&["pr"]), no_env).unwrap(),
            json!({"key": "pr"})
        );
        assert_eq!(
            build_clear_status_params(&args(&[]), no_env).unwrap(),
            json!({})
        );
        assert_eq!(
            build_set_description_params(&args(&["PROD", "DEPLOYS", "PENDING"]), no_env).unwrap(),
            json!({"description": "PROD DEPLOYS PENDING"})
        );
        assert_eq!(
            build_set_description_params(&args(&["--clear"]), no_env).unwrap(),
            json!({"description": ""})
        );
        assert!(build_set_description_params(&args(&[]), no_env).is_err());
    }

    #[test]
    fn lifecycle_follows_cmux() {
        let (method, params) = lifecycle_params("claude", "UserPromptSubmit", "ws").unwrap();
        assert_eq!(method, "workspace.set_status");
        assert_eq!(params["value"], "Running");
        assert_eq!(
            lifecycle_params("claude", "Notification", "ws").unwrap().1["value"],
            "Needs input"
        );
        let (method, params) = lifecycle_params("claude", "stop", "ws").unwrap();
        assert_eq!(method, "workspace.clear_status");
        assert_eq!(params["key"], "claude");
        let (method, params) = lifecycle_params("claude", "SessionEnd", "ws").unwrap();
        assert_eq!(method, "workspace.clear_status");
        assert_eq!(params["key"], "claude");
        assert!(lifecycle_params("claude", "SubagentStop", "ws").is_none());
    }

    #[test]
    fn gh_answers_map_to_pr_rows() {
        assert_eq!(
            parse_gh_pr_view(true, br#"{"number":503,"state":"OPEN"}"#, b""),
            PrLookup::Found {
                number: 503,
                state: "open".into()
            }
        );
        assert_eq!(
            parse_gh_pr_view(false, b"", b"no pull requests found for branch \"x\""),
            PrLookup::NoPr
        );
        assert_eq!(
            parse_gh_pr_view(false, b"", b"HTTP 401"),
            PrLookup::Unavailable
        );
        let (_, params) = pr_status_params(
            &PrLookup::Found {
                number: 503,
                state: "open".into(),
            },
            "ws",
        )
        .unwrap();
        assert_eq!(params["value"], "PR #503 open");
        assert_eq!(
            pr_status_params(&PrLookup::NoPr, "ws").unwrap().0,
            "workspace.clear_status"
        );
        assert!(pr_status_params(&PrLookup::Unavailable, "ws").is_none());
    }
}
