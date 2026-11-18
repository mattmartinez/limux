//! OSC 7501 program status on the workspace card, the way cmux shows it.
//!
//! A program writes its state to the terminal (`OSC 7501 ; state=working :
//! app=omp ...`); Ghostty parses it and hands it to Limux as an action. Each
//! terminal tab keeps its own records, one per record id, and the card shows
//! the most urgent record of the workspace as one status row:
//!
//! - order: blocked (bell), error, working (with its percentage), done; idle
//!   records are kept but not shown;
//! - text: the record's app (inherited from the nearest parent id), then its
//!   message, or title, or a label such as "Needs permission", joined with
//!   " · ", as in "omp · Working";
//! - working and blocked records go when a new shell prompt starts or the
//!   tab's process exits; done and error records stay until the user focuses
//!   that tab again; a clear report removes a record and the ids under it, or
//!   every record when it has no id.
//!
//! Ported from cmux's `ProgramStatusRecordStore` and `Workspace+ProgramStatus`.
//! Plain data, tested without GTK.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::workspace_card::{StatusEntry, MAX_VALUE_LEN};

/// The status key of the program status row.
pub const STATUS_KEY: &str = "program_status";
/// Most records one tab keeps; the oldest goes first.
const RECORD_LIMIT: usize = 256;
/// Longest message or title kept, in characters (cmux's limit).
const MAX_TEXT_LEN: usize = 512;
/// The colour of a blocked row (cmux's).
const BLOCKED_COLOR: &str = "#4C8DFF";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    None,
    Permission,
    Question,
    Auth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A program's report.
    Report,
    /// A new shell prompt started (OSC 133 A), or the tab's process exited.
    PromptStart,
}

/// One report from a tab's terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub event: Event,
    pub state: State,
    pub kind: Kind,
    pub progress: Option<u8>,
    pub id: Option<String>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub msg: Option<String>,
}

impl Report {
    /// A prompt start: working and blocked records end.
    pub fn prompt_start() -> Self {
        Self {
            event: Event::PromptStart,
            state: State::Idle,
            kind: Kind::None,
            progress: None,
            id: None,
            app: None,
            title: None,
            msg: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    id: String,
    state: State,
    kind: Kind,
    progress: Option<u8>,
    app: Option<String>,
    title: Option<String>,
    msg: Option<String>,
    /// When the record was written, across every store, so rows of different
    /// tabs compare by age.
    order: u64,
}

static NEXT_ORDER: AtomicU64 = AtomicU64::new(1);

/// The records of one terminal tab, keyed by record id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    records: BTreeMap<String, Record>,
}

impl Store {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn apply(&mut self, report: Report) {
        if report.event == Event::PromptStart {
            self.drop_transient();
            return;
        }
        if report.state == State::Clear {
            match report.id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => {
                    let nested = format!("{id}/");
                    self.records
                        .retain(|key, _| key != id && !key.starts_with(&nested));
                }
                None => self.records.clear(),
            }
            return;
        }
        let id = report.id.unwrap_or_default();
        let record = Record {
            id: id.clone(),
            state: report.state,
            kind: if report.state == State::Blocked {
                report.kind
            } else {
                Kind::None
            },
            progress: matches!(report.state, State::Working | State::Blocked)
                .then_some(report.progress)
                .flatten(),
            app: clean_text(report.app.as_deref()),
            title: clean_text(report.title.as_deref()),
            msg: clean_text(report.msg.as_deref()),
            order: NEXT_ORDER.fetch_add(1, Ordering::Relaxed),
        };
        self.records.insert(id, record);
        if self.records.len() > RECORD_LIMIT {
            if let Some(oldest) = self
                .records
                .values()
                .min_by_key(|record| record.order)
                .map(|record| record.id.clone())
            {
                self.records.remove(&oldest);
            }
        }
    }

    /// Working and blocked records end at a prompt or process exit.
    pub fn drop_transient(&mut self) {
        self.records
            .retain(|_, record| !matches!(record.state, State::Working | State::Blocked));
    }

    /// Done and error records end once the user is back in the tab. Returns
    /// whether any went.
    pub fn dismiss_completed(&mut self) -> bool {
        let before = self.records.len();
        self.records
            .retain(|_, record| !matches!(record.state, State::Done | State::Error));
        self.records.len() != before
    }

    fn effective_app<'a>(&'a self, record: &'a Record) -> Option<&'a str> {
        if let Some(app) = record.app.as_deref() {
            return Some(app);
        }
        let mut parent = record.id.as_str();
        while let Some(slash) = parent.rfind('/') {
            parent = &parent[..slash];
            if let Some(app) = self.records.get(parent).and_then(|r| r.app.as_deref()) {
                return Some(app);
            }
        }
        self.records.get("").and_then(|r| r.app.as_deref())
    }

    fn most_urgent(&self) -> Option<&Record> {
        self.records
            .values()
            .filter(|record| record.state != State::Idle)
            .max_by_key(|record| (urgency(record.state), record.order))
    }

    /// This tab's row and when its record was written.
    fn row(&self) -> Option<(StatusEntry, u64)> {
        let record = self.most_urgent()?;
        let (icon, fallback) = match record.state {
            State::Blocked => (
                "\u{1F514}",
                match record.kind {
                    Kind::Permission => "Needs permission",
                    Kind::Question => "Needs an answer",
                    Kind::Auth => "Needs sign-in",
                    Kind::None => "Needs input",
                },
            ),
            State::Working => ("\u{26A1}", "Working"),
            State::Done => ("\u{2713}", "Done"),
            State::Error => ("\u{26A0}", "Failed"),
            State::Idle | State::Clear => return None,
        };
        let detail = record
            .msg
            .as_deref()
            .or(record.title.as_deref())
            .unwrap_or(fallback);
        let percent = record.progress.map(|progress| format!("{progress}%"));
        let value = [self.effective_app(record), Some(detail), percent.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        Some((
            StatusEntry {
                key: STATUS_KEY.to_string(),
                value: shorten(&value, MAX_VALUE_LEN),
                icon: Some(icon.to_string()),
                color: (record.state == State::Blocked).then(|| BLOCKED_COLOR.to_string()),
                priority: urgency(record.state),
            },
            record.order,
        ))
    }
}

/// How urgent a state is; the row's priority.
pub fn urgency(state: State) -> i64 {
    match state {
        State::Blocked => 4,
        State::Error => 3,
        State::Working => 2,
        State::Done => 1,
        State::Idle | State::Clear => 0,
    }
}

/// The row a workspace shows: the most urgent row of its tabs, the newest
/// among equals. `None` when no tab has one.
pub fn workspace_row<'a>(stores: impl IntoIterator<Item = &'a Store>) -> Option<StatusEntry> {
    stores
        .into_iter()
        .filter_map(Store::row)
        .max_by_key(|(entry, order)| (entry.priority, *order))
        .map(|(entry, _)| entry)
}

/// Text without invisible formatting characters (direction overrides,
/// zero-width marks and the like) or controls, trimmed and cut to 512
/// characters; `None` when nothing is left.
fn clean_text(text: Option<&str>) -> Option<String> {
    let kept: String = text?
        .chars()
        .filter(|&c| !c.is_control() && !is_format_char(c))
        .collect();
    let trimmed = kept.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(MAX_TEXT_LEN).collect())
}

/// Unicode's format characters (general category Cf).
fn is_format_char(c: char) -> bool {
    matches!(
        u32::from(c),
        0xAD | 0x600..=0x605
            | 0x61C
            | 0x6DD
            | 0x70F
            | 0x890..=0x891
            | 0x8E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('\u{2026}');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(state: State) -> Report {
        Report {
            event: Event::Report,
            state,
            kind: Kind::None,
            progress: None,
            id: None,
            app: None,
            title: None,
            msg: None,
        }
    }

    fn omp(state: State) -> Report {
        Report {
            app: Some("omp".to_string()),
            ..report(state)
        }
    }

    fn row(store: &Store) -> Option<String> {
        workspace_row([store]).map(|entry| {
            format!(
                "{} {} p{}{}",
                entry.icon.unwrap_or_default(),
                entry.value,
                entry.priority,
                entry.color.map(|c| format!(" {c}")).unwrap_or_default()
            )
        })
    }

    #[test]
    fn rows_read_like_cmux() {
        let mut store = Store::default();
        store.apply(omp(State::Working));
        assert_eq!(row(&store).as_deref(), Some("\u{26A1} omp · Working p2"));
        store.apply(omp(State::Done));
        assert_eq!(row(&store).as_deref(), Some("\u{2713} omp · Done p1"));
        store.apply(Report {
            kind: Kind::Permission,
            ..omp(State::Blocked)
        });
        assert_eq!(
            row(&store).as_deref(),
            Some("\u{1F514} omp · Needs permission p4 #4C8DFF")
        );
        store.apply(Report {
            msg: Some("rsync failed".to_string()),
            ..omp(State::Error)
        });
        assert_eq!(
            row(&store).as_deref(),
            Some("\u{26A0} omp · rsync failed p3")
        );
        store.apply(Report {
            progress: Some(40),
            title: Some("Syncing photos".to_string()),
            ..report(State::Working)
        });
        assert_eq!(
            row(&store).as_deref(),
            Some("\u{26A1} Syncing photos · 40% p2")
        );
        store.apply(report(State::Idle));
        assert_eq!(row(&store), None);
    }

    #[test]
    fn the_most_urgent_record_wins_then_the_newest() {
        let mut store = Store::default();
        let with_id = |id: &str, state| Report {
            id: Some(id.to_string()),
            msg: Some(id.to_string()),
            ..report(state)
        };
        store.apply(with_id("a", State::Done));
        store.apply(with_id("b", State::Working));
        assert_eq!(row(&store).as_deref(), Some("\u{26A1} b p2"));
        store.apply(with_id("c", State::Blocked));
        assert_eq!(row(&store).as_deref(), Some("\u{1F514} c p4 #4C8DFF"));
        store.apply(with_id("d", State::Blocked));
        assert_eq!(row(&store).as_deref(), Some("\u{1F514} d p4 #4C8DFF"));
    }

    #[test]
    fn a_child_record_inherits_its_parents_app() {
        let mut store = Store::default();
        store.apply(Report {
            id: Some("build".to_string()),
            ..omp(State::Idle)
        });
        store.apply(Report {
            id: Some("build/test".to_string()),
            ..report(State::Working)
        });
        assert_eq!(row(&store).as_deref(), Some("\u{26A1} omp · Working p2"));
    }

    #[test]
    fn a_prompt_ends_working_and_blocked_but_keeps_done() {
        let mut store = Store::default();
        store.apply(Report {
            id: Some("x".to_string()),
            ..omp(State::Working)
        });
        store.apply(Report {
            id: Some("y".to_string()),
            ..omp(State::Done)
        });
        store.apply(Report::prompt_start());
        assert_eq!(row(&store).as_deref(), Some("\u{2713} omp · Done p1"));
        assert!(store.dismiss_completed());
        assert!(store.is_empty());
        assert!(!store.dismiss_completed());
    }

    #[test]
    fn clear_removes_one_id_and_its_children_or_everything() {
        let mut store = Store::default();
        for id in ["a", "a/b", "ab", "c"] {
            store.apply(Report {
                id: Some(id.to_string()),
                ..omp(State::Working)
            });
        }
        store.apply(Report {
            id: Some("a".to_string()),
            ..report(State::Clear)
        });
        assert_eq!(
            store.records.keys().cloned().collect::<Vec<_>>(),
            ["ab", "c"]
        );
        store.apply(report(State::Clear));
        assert!(store.is_empty());
    }

    #[test]
    fn progress_and_kind_only_count_where_cmux_counts_them() {
        let mut store = Store::default();
        store.apply(Report {
            progress: Some(80),
            kind: Kind::Question,
            ..omp(State::Done)
        });
        assert_eq!(row(&store).as_deref(), Some("\u{2713} omp · Done p1"));
    }

    #[test]
    fn invisible_and_control_text_is_dropped() {
        let mut store = Store::default();
        store.apply(Report {
            msg: Some("\u{202e}evil\u{200b} \u{7}name ".to_string()),
            app: Some("  ".to_string()),
            ..report(State::Working)
        });
        assert_eq!(row(&store).as_deref(), Some("\u{26A1} evil name p2"));
    }

    #[test]
    fn the_workspace_shows_the_most_urgent_tab_then_the_newest() {
        let mut first = Store::default();
        let mut second = Store::default();
        first.apply(omp(State::Working));
        second.apply(Report {
            app: Some("claude-code".to_string()),
            ..report(State::Working)
        });
        let value = |stores: [&Store; 2]| workspace_row(stores).map(|entry| entry.value);
        assert_eq!(
            value([&first, &second]).as_deref(),
            Some("claude-code · Working")
        );
        first.apply(omp(State::Blocked));
        assert_eq!(
            value([&first, &second]).as_deref(),
            Some("omp · Needs input")
        );
        assert_eq!(workspace_row([&Store::default()]), None);
    }

    #[test]
    fn records_are_capped_oldest_first() {
        let mut store = Store::default();
        for n in 0..=RECORD_LIMIT {
            store.apply(Report {
                id: Some(format!("r{n:04}")),
                ..report(State::Done)
            });
        }
        assert_eq!(store.records.len(), RECORD_LIMIT);
        assert!(!store.records.contains_key("r0000"));
    }

    #[test]
    fn long_text_is_shortened_for_the_card() {
        let mut store = Store::default();
        store.apply(Report {
            msg: Some("x".repeat(600)),
            ..report(State::Working)
        });
        let entry = workspace_row([&store]).unwrap();
        assert_eq!(entry.value.chars().count(), MAX_VALUE_LEN);
        assert!(entry.value.ends_with('\u{2026}'));
    }
}
