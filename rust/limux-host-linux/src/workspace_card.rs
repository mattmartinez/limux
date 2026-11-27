//! Agent-written state shown on a sidebar workspace card.
//!
//! Agents, hooks and scripts write it over the control socket, the way cmux
//! cards work: `workspace.set_status` adds or replaces one keyed status row,
//! `workspace.clear_status` drops one or all of them, `workspace.list_status`
//! reads them back, and `workspace.set_description` sets a free-text line
//! under the title. The card also shows the git branch of the workspace's
//! current directory, read from `.git/HEAD` without running git.
//!
//! Everything here is plain data and validation, so it is tested without GTK.
//! Status rows live only in memory: they describe a running agent and are
//! meaningless after a restart. The description is the user's and is saved
//! with the session.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Most status rows one workspace keeps; a new key beyond this is refused.
pub const MAX_STATUS_ENTRIES: usize = 16;
/// Longest status key, in characters.
pub const MAX_KEY_LEN: usize = 32;
/// Longest status value, in characters.
pub const MAX_VALUE_LEN: usize = 160;
/// Longest description, in characters.
pub const MAX_DESCRIPTION_LEN: usize = 200;
/// Highest absolute priority accepted; rows sort by priority, highest first.
pub const MAX_PRIORITY: i64 = 1000;

/// CSS class on the box that holds a card's status rows.
pub const STATUS_BOX_CSS_CLASS: &str = "limux-ws-status";
/// CSS class on each status row label.
pub const STATUS_ROW_CSS_CLASS: &str = "limux-ws-status-row";
/// CSS class on the description label.
pub const DESCRIPTION_CSS_CLASS: &str = "limux-ws-description";
/// CSS class on the git branch label in the path row.
pub const BRANCH_CSS_CLASS: &str = "limux-ws-branch";

/// One keyed row on a card, for example `claude: Running`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusEntry {
    pub key: String,
    pub value: String,
    /// A short glyph shown before the value. cmux's SF Symbol names such as
    /// `bolt.fill` are mapped to a Unicode glyph; anything else of at most
    /// four characters is shown as it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `#rrggbb`, applied to the icon (or to a leading dot when there is no icon).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub priority: i64,
    /// Built-in lifecycle and OSC rows compete for one activity line.
    #[serde(skip)]
    pub agent_activity: bool,
}

/// The status rows of one workspace, keyed by status key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusMap {
    entries: BTreeMap<String, StatusEntry>,
}

impl StatusMap {
    /// Add or replace the row with this entry's key. Refuses a new key when
    /// the map already holds `MAX_STATUS_ENTRIES` rows.
    pub fn set(&mut self, entry: StatusEntry) -> Result<(), String> {
        if !self.entries.contains_key(&entry.key) && self.entries.len() >= MAX_STATUS_ENTRIES {
            return Err(format!(
                "a workspace holds at most {MAX_STATUS_ENTRIES} status rows; clear one first"
            ));
        }
        self.entries.insert(entry.key.clone(), entry);
        Ok(())
    }

    /// Drop one row, or every row when `key` is `None`. Returns how many went.
    pub fn clear(&mut self, key: Option<&str>) -> usize {
        match key {
            Some(key) => usize::from(self.entries.remove(key).is_some()),
            None => {
                let count = self.entries.len();
                self.entries.clear();
                count
            }
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Rows in display order: highest priority first, then by key.
    pub fn sorted(&self) -> Vec<&StatusEntry> {
        let mut rows: Vec<&StatusEntry> = self.entries.values().collect();
        rows.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.key.cmp(&b.key)));
        rows
    }

    pub fn displayed(&self) -> impl Iterator<Item = &StatusEntry> {
        let blocked = crate::program_status::urgency(crate::program_status::State::Blocked);
        let activity = self
            .entries
            .values()
            .filter(|entry| entry.agent_activity)
            .max_by_key(|entry| {
                (
                    entry.priority >= blocked,
                    entry.key == crate::program_status::STATUS_KEY,
                    entry.priority,
                )
            })
            .map(|entry| entry.key.as_str());
        self.sorted()
            .into_iter()
            .filter(move |entry| !entry.agent_activity || Some(entry.key.as_str()) == activity)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.sorted()
                .into_iter()
                .map(|entry| serde_json::to_value(entry).unwrap_or(serde_json::Value::Null))
                .collect(),
        )
    }
}

/// What a card request asks the host to do to one workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardRequest {
    SetStatus(StatusEntry),
    ClearStatus(Option<String>),
    ListStatus,
    SetDescription(Option<String>),
}

/// Parse the params of one of the `workspace.*status` / `set_description`
/// methods. The workspace target is parsed separately by the bridge.
pub fn parse_card_request(
    method: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<CardRequest, String> {
    match method {
        "workspace.set_status" => {
            let key = parse_key(required_str(params, "key", method)?)?;
            let value = parse_text(
                required_str(params, "value", method)?,
                "value",
                MAX_VALUE_LEN,
            )?
            .ok_or_else(|| {
                format!(
                    "{method} needs a non-empty value; use workspace.clear_status to remove a row"
                )
            })?;
            let icon = optional_str(params, "icon", method)?
                .map(parse_icon)
                .transpose()?
                .flatten();
            let color = optional_str(params, "color", method)?
                .map(parse_color)
                .transpose()?;
            let priority = match params.get("priority") {
                None | Some(serde_json::Value::Null) => 0,
                Some(value) => value
                    .as_i64()
                    .filter(|p| p.abs() <= MAX_PRIORITY)
                    .ok_or_else(|| format!("{method}: priority must be an integer from -{MAX_PRIORITY} to {MAX_PRIORITY}"))?,
            };
            Ok(CardRequest::SetStatus(StatusEntry {
                key,
                value,
                icon,
                color,
                priority,
                agent_activity: params
                    .get("agent_activity")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            }))
        }
        "workspace.clear_status" => {
            let key = optional_str(params, "key", method)?
                .map(parse_key)
                .transpose()?;
            Ok(CardRequest::ClearStatus(key))
        }
        "workspace.list_status" => Ok(CardRequest::ListStatus),
        "workspace.set_description" => {
            let text = optional_str(params, "description", method)?.unwrap_or("");
            Ok(CardRequest::SetDescription(parse_text(
                text,
                "description",
                MAX_DESCRIPTION_LEN,
            )?))
        }
        other => Err(format!("{other} is not a workspace card method")),
    }
}

fn required_str<'a>(
    params: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
    method: &str,
) -> Result<&'a str, String> {
    optional_str(params, name, method)?.ok_or_else(|| format!("{method} requires `{name}`"))
}

fn optional_str<'a>(
    params: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
    method: &str,
) -> Result<Option<&'a str>, String> {
    match params.get(name) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(format!("{method}: `{name}` must be a string")),
    }
}

/// Keys are short identifiers so rows from different agents never collide by accident.
fn parse_key(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    let valid = !key.is_empty()
        && key.chars().count() <= MAX_KEY_LEN
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if valid {
        Ok(key.to_string())
    } else {
        Err(format!(
            "status key must be 1 to {MAX_KEY_LEN} letters, digits, '_', '-' or '.'"
        ))
    }
}

/// One line of display text: control characters become spaces, runs of
/// whitespace collapse, and the length is capped. Empty means "none".
fn parse_text(raw: &str, what: &str, max: usize) -> Result<Option<String>, String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok((!cleaned.is_empty()).then_some(cleaned))
}

/// `#rrggbb` (or `rrggbb`), or one of a few plain names.
pub fn parse_color(raw: &str) -> Result<String, String> {
    let named = match raw.trim().to_ascii_lowercase().as_str() {
        "red" => Some("#e5484d"),
        "orange" => Some("#e8955a"),
        "amber" | "yellow" => Some("#e2c05c"),
        "green" => Some("#57c785"),
        "teal" | "cyan" => Some("#4fc1d4"),
        "blue" => Some("#5c9cf5"),
        "purple" => Some("#b889f4"),
        "pink" => Some("#f47fb5"),
        "gray" | "grey" => Some("#8d8d8d"),
        _ => None,
    };
    if let Some(hex) = named {
        return Ok(hex.to_string());
    }
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(format!("#{}", hex.to_ascii_lowercase()))
    } else {
        Err("color must be #rrggbb or a name such as red, amber, green or blue".to_string())
    }
}

/// Map cmux's SF Symbol names to a glyph; pass short text through.
fn parse_icon(raw: &str) -> Result<Option<String>, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let mapped = match name {
        "bolt.fill" | "bolt" => Some("\u{26A1}"),
        "pause.circle.fill" | "pause.circle" | "pause" => Some("\u{23F8}"),
        "bell.fill" | "bell" | "bell.badge.fill" => Some("\u{1F514}"),
        "checkmark.circle.fill" | "checkmark.circle" | "checkmark" => Some("\u{2713}"),
        "xmark.circle.fill" | "xmark.circle" | "xmark" => Some("\u{2717}"),
        "exclamationmark.triangle.fill" | "exclamationmark.triangle" => Some("\u{26A0}"),
        "hourglass" | "clock" | "clock.fill" => Some("\u{231B}"),
        "arrow.triangle.pull" | "arrow.triangle.branch" => Some("\u{2442}"),
        "circle.fill" | "circle" => Some("\u{25CF}"),
        _ => None,
    };
    if let Some(glyph) = mapped {
        return Ok(Some(glyph.to_string()));
    }
    if name.chars().count() <= 4 && !name.chars().any(char::is_control) {
        Ok(Some(name.to_string()))
    } else {
        Err(format!(
            "unknown icon `{name}`: use a known SF Symbol name or up to 4 characters"
        ))
    }
}

/// Pango markup for one status row: a coloured icon or dot, then the value.
/// Every piece of agent-supplied text is escaped.
pub fn status_row_markup(entry: &StatusEntry) -> String {
    let value = gtk_free_escape(&entry.value);
    let glyph = entry
        .icon
        .as_deref()
        .map(gtk_free_escape)
        .or_else(|| entry.color.as_ref().map(|_| "\u{25CF}".to_string()));
    match (glyph, entry.color.as_deref()) {
        (Some(glyph), Some(color)) => {
            format!("<span foreground=\"{color}\">{glyph}</span> {value}")
        }
        (Some(glyph), None) => format!("{glyph} {value}"),
        (None, _) => value,
    }
}

/// Escape text for Pango markup. Same rules as `glib::markup_escape_text`,
/// kept here so this module needs no GTK to be tested.
fn gtk_free_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The branch checked out in the repository that contains `dir`, read from
/// `.git/HEAD` (or the HEAD a worktree's `.git` file points to). `None` when
/// `dir` is not in a repository; a detached HEAD gives its short commit.
pub fn git_branch_for(dir: &Path) -> Option<String> {
    let head = head_file_for(dir)?;
    let content = std::fs::read_to_string(head).ok()?;
    let content = content.trim();
    if let Some(reference) = content.strip_prefix("ref:") {
        let reference = reference.trim();
        let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    (content.len() >= 7 && content.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| content[..7].to_string())
}

fn head_file_for(dir: &Path) -> Option<PathBuf> {
    for candidate in dir.ancestors().take(64) {
        let dot_git = candidate.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git.join("HEAD"));
        }
        if dot_git.is_file() {
            let pointer = std::fs::read_to_string(&dot_git).ok()?;
            let git_dir = pointer.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = Path::new(git_dir);
            let git_dir = if git_dir.is_absolute() {
                git_dir.to_path_buf()
            } else {
                candidate.join(git_dir)
            };
            return Some(git_dir.join("HEAD"));
        }
    }
    None
}

/// Longest workspace name taken from a terminal title, in characters with the
/// ellipsis (cmux's limit).
const MAX_TERMINAL_TITLE_CHARS: usize = 256;

/// The workspace name a terminal title gives, by cmux's rules
/// (`AutomaticTerminalTitle` and `TerminalTitleChurnFilter`):
/// - line and tab breaks become spaces;
/// - a title with any other control or bidi-override character is refused;
/// - it is cut to 256 characters;
/// - a spinner frame standing alone as a word is dropped, so an agent's
///   animated title settles on one name. Two or more glyphs in a row are
///   art, not a spinner, and stay.
///
/// `None` when nothing usable is left.
pub fn workspace_title_from_terminal(raw: &str) -> Option<String> {
    let mut chars = Vec::new();
    for c in raw.chars().take(MAX_TERMINAL_TITLE_CHARS + 1) {
        match u32::from(c) {
            0x09..=0x0D | 0x85 | 0x2028 | 0x2029 => chars.push(' '),
            0x00..=0x1F | 0x7F..=0x9F | 0x202A..=0x202E | 0x2066..=0x2069 => return None,
            _ => chars.push(c),
        }
    }
    if chars.len() > MAX_TERMINAL_TITLE_CHARS {
        chars.truncate(MAX_TERMINAL_TITLE_CHARS - 1);
        chars.push('\u{2026}');
    }

    let glyph_run = chars
        .windows(2)
        .any(|pair| is_spinner_frame(pair[0]) && is_spinner_frame(pair[1]));
    let standalone = |index: usize| {
        (index == 0 || chars[index - 1].is_whitespace())
            && chars.get(index + 1).is_none_or(|next| next.is_whitespace())
    };
    let kept: Vec<char> = chars
        .iter()
        .enumerate()
        .filter(|&(index, &c)| glyph_run || !(is_spinner_frame(c) && standalone(index)))
        .map(|(_, &c)| c)
        .collect();
    let title: String = if kept.len() == chars.len() {
        kept.iter().collect::<String>().trim().to_string()
    } else {
        kept.iter()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    (!title.is_empty()).then_some(title)
}

/// A spinner frame: cmux's ✱..✽ stars and quarter-circle spinners, and any
/// braille pattern. cmux names ten braille frames; a braille cell standing
/// alone in a title is a spinner frame whichever set a program animates.
fn is_spinner_frame(c: char) -> bool {
    matches!(
        u32::from(c),
        0x2800..=0x28FF | 0x2731..=0x273D | 0x25D0..=0x25D3 | 0x25F4..=0x25F7
    )
}

/// CSS for the card rows. Secondary lines match the path; selected rows go white
/// so they stay readable on the blue selection fill.
pub fn card_css() -> &'static str {
    ".limux-ws-description,\n\
     .limux-ws-status-row,\n\
     .limux-ws-branch {\n\
         font-size: 0.85em;\n\
         color: alpha(white, 0.60);\n\
     }\n\
     .limux-ws-branch { margin-right: 6px; }\n\
     .limux-sidebar-list row:selected .limux-ws-description,\n\
     .limux-sidebar-list row:selected .limux-ws-status-row,\n\
     .limux-sidebar-list row:selected .limux-ws-branch {\n\
         color: white;\n\
     }\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terminal_titles_become_workspace_names_as_cmux_admits_them() {
        let name = workspace_title_from_terminal;
        assert_eq!(name("π > demo-repo").as_deref(), Some("π > demo-repo"));
        assert_eq!(name("~/Work/demo").as_deref(), Some("~/Work/demo"));
        assert_eq!(name("  vim  notes.md ").as_deref(), Some("vim  notes.md"));
        assert_eq!(name("make\ttest\nnow").as_deref(), Some("make test now"));
        assert_eq!(name("red \u{1b}[31m"), None);
        assert_eq!(name("abc \u{202e}cba"), None);
        assert_eq!(name("   "), None);
        let long = name(&"x".repeat(300)).expect("a long title is cut, not refused");
        assert_eq!(long.chars().count(), 256);
        assert!(long.ends_with('\u{2026}'));
        assert_eq!(
            name(&"x".repeat(256)).as_deref(),
            Some("x".repeat(256).as_str())
        );
    }

    #[test]
    fn an_agents_spinner_frames_settle_on_one_workspace_name() {
        let name = workspace_title_from_terminal;
        for frame in ["✳", "✶", "⠂", "⠐", "⠋", "◐"] {
            assert_eq!(
                name(&format!("{frame} Fix the card title")).as_deref(),
                Some("Fix the card title"),
                "frame {frame}"
            );
        }
        assert_eq!(name("Fix it ⠙").as_deref(), Some("Fix it"));
        assert_eq!(name("✳").as_deref(), None);
        // Glyphs inside a word or in a row are text, not a spinner.
        assert_eq!(name("a✳b").as_deref(), Some("a✳b"));
        assert_eq!(name("⠋⠙ braille art").as_deref(), Some("⠋⠙ braille art"));
    }

    fn parse(method: &str, params: serde_json::Value) -> Result<CardRequest, String> {
        parse_card_request(method, params.as_object().expect("test params are objects"))
    }

    fn entry(key: &str, priority: i64) -> StatusEntry {
        StatusEntry {
            key: key.into(),
            value: format!("{key} value"),
            icon: None,
            color: None,
            priority,
            agent_activity: false,
        }
    }

    #[test]
    fn set_status_parses_cmux_style_params() {
        let request = parse(
            "workspace.set_status",
            json!({"key": "claude", "value": "Running", "icon": "bolt.fill", "color": "#4C8DFF", "priority": 5}),
        )
        .unwrap();
        assert_eq!(
            request,
            CardRequest::SetStatus(StatusEntry {
                key: "claude".into(),
                value: "Running".into(),
                icon: Some("\u{26A1}".into()),
                color: Some("#4c8dff".into()),
                priority: 5,
                agent_activity: false,
            })
        );
    }

    #[test]
    fn set_status_rejects_bad_input() {
        let bad = [
            json!({"value": "x"}),
            json!({"key": "has space", "value": "x"}),
            json!({"key": "k", "value": "   "}),
            json!({"key": "k", "value": "x", "color": "not-a-colour"}),
            json!({"key": "k", "value": "x", "priority": 5000}),
            json!({"key": "k", "value": "x", "icon": "a.very.long.unknown.symbol"}),
            json!({"key": "k", "value": "x".repeat(MAX_VALUE_LEN + 1)}),
            json!({"key": 7, "value": "x"}),
        ];
        for params in bad {
            assert!(
                parse("workspace.set_status", params.clone()).is_err(),
                "{params}"
            );
        }
    }

    #[test]
    fn text_is_one_clean_line() {
        let request = parse(
            "workspace.set_status",
            json!({"key": "k", "value": "  needs\n\tyour   input\u{7} "}),
        )
        .unwrap();
        let CardRequest::SetStatus(entry) = request else {
            panic!()
        };
        assert_eq!(entry.value, "needs your input");
    }

    #[test]
    fn description_empty_clears() {
        assert_eq!(
            parse("workspace.set_description", json!({"description": "  "})).unwrap(),
            CardRequest::SetDescription(None)
        );
        assert_eq!(
            parse("workspace.set_description", json!({})).unwrap(),
            CardRequest::SetDescription(None)
        );
        assert_eq!(
            parse(
                "workspace.set_description",
                json!({"description": "PROD DEPLOYS PENDING"})
            )
            .unwrap(),
            CardRequest::SetDescription(Some("PROD DEPLOYS PENDING".into()))
        );
    }

    #[test]
    fn clear_status_one_or_all() {
        assert_eq!(
            parse("workspace.clear_status", json!({"key": "pr"})).unwrap(),
            CardRequest::ClearStatus(Some("pr".into()))
        );
        assert_eq!(
            parse("workspace.clear_status", json!({})).unwrap(),
            CardRequest::ClearStatus(None)
        );
        let mut map = StatusMap::default();
        map.set(entry("a", 0)).unwrap();
        map.set(entry("b", 0)).unwrap();
        assert_eq!(map.clear(Some("a")), 1);
        assert_eq!(map.clear(Some("a")), 0);
        assert_eq!(map.clear(None), 1);
        assert!(map.is_empty());
    }

    #[test]
    fn rows_sort_by_priority_then_key_and_replace_by_key() {
        let mut map = StatusMap::default();
        map.set(entry("pr", -1)).unwrap();
        map.set(entry("claude", 2)).unwrap();
        map.set(entry("aaa", 2)).unwrap();
        map.set(StatusEntry {
            value: "Idle".into(),
            ..entry("claude", 2)
        })
        .unwrap();
        let keys: Vec<&str> = map.sorted().iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["aaa", "claude", "pr"]);
        assert_eq!(map.sorted()[1].value, "Idle");
    }

    #[test]
    fn card_prefers_one_osc_activity_row_and_keeps_custom_statuses() {
        let mut map = StatusMap::default();
        map.set(StatusEntry {
            agent_activity: true,
            ..entry("claude", 2)
        })
        .unwrap();
        map.set(StatusEntry {
            agent_activity: true,
            ..entry(crate::program_status::STATUS_KEY, 2)
        })
        .unwrap();
        map.set(entry("build", 20)).unwrap();
        map.set(entry("pr", -1)).unwrap();
        let keys = |map: &StatusMap| {
            map.displayed()
                .map(|row| row.key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            keys(&map),
            ["build", crate::program_status::STATUS_KEY, "pr"]
        );
        map.clear(Some(crate::program_status::STATUS_KEY));
        assert_eq!(keys(&map), ["build", "claude", "pr"]);
        map.set(StatusEntry {
            agent_activity: true,
            ..entry(crate::program_status::STATUS_KEY, 2)
        })
        .unwrap();
        // A permission hook still wins over a different program that's working.
        map.set(StatusEntry {
            agent_activity: true,
            ..entry("claude", 4)
        })
        .unwrap();
        assert_eq!(keys(&map), ["build", "claude", "pr"]);
    }

    #[test]
    fn row_count_is_capped_but_replacing_is_allowed() {
        let mut map = StatusMap::default();
        for i in 0..MAX_STATUS_ENTRIES {
            map.set(entry(&format!("k{i}"), 0)).unwrap();
        }
        assert!(map.set(entry("one-more", 0)).is_err());
        assert!(map.set(entry("k0", 9)).is_ok());
    }

    #[test]
    fn markup_escapes_agent_text() {
        let row = StatusEntry {
            key: "k".into(),
            value: "<img src=x> & \"q\"".into(),
            icon: Some("<b>".into()),
            color: Some("#e5484d".into()),
            priority: 0,
            agent_activity: false,
        };
        let markup = status_row_markup(&row);
        assert_eq!(
            markup,
            "<span foreground=\"#e5484d\">&lt;b&gt;</span> &lt;img src=x&gt; &amp; &quot;q&quot;"
        );
        let plain = StatusEntry {
            icon: None,
            color: None,
            ..row.clone()
        };
        assert!(!status_row_markup(&plain).contains('<'));
        let dot = StatusEntry { icon: None, ..row };
        assert!(status_row_markup(&dot).starts_with("<span foreground=\"#e5484d\">\u{25CF}</span>"));
    }

    #[test]
    fn git_branch_reads_head_dirs_and_worktree_files() {
        let root = std::env::temp_dir().join(format!("limux-card-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/fix-123\n").unwrap();
        assert_eq!(
            git_branch_for(&repo.join("src/deep")).as_deref(),
            Some("fix-123")
        );

        let worktree = root.join("wt");
        let wt_git = root.join("repo/.git/worktrees/wt");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(
            wt_git.join("HEAD"),
            "3f1c2d9e8b7a6f5e4d3c2b1a0f9e8d7c6b5a4f3e\n",
        )
        .unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", wt_git.display()),
        )
        .unwrap();
        assert_eq!(git_branch_for(&worktree).as_deref(), Some("3f1c2d9"));

        let outside = root.join("plain");
        std::fs::create_dir_all(&outside).unwrap();
        // The temp dir itself is not inside a repository on any sane system.
        let found = git_branch_for(&outside);
        let _ = std::fs::remove_dir_all(&root);
        assert!(found.is_none() || std::path::Path::new("/.git").exists());
    }

    #[test]
    fn colors_accept_hex_and_names() {
        assert_eq!(parse_color("#ABCDEF").unwrap(), "#abcdef");
        assert_eq!(parse_color("abcdef").unwrap(), "#abcdef");
        assert_eq!(parse_color("Amber").unwrap(), "#e2c05c");
        assert!(parse_color("#abcd").is_err());
    }
}
