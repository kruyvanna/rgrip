//! Server sessions for the Activity tab: the `pg_stat_activity` query, its
//! rows, and the classification (blocked, blocking, long-running, idle in
//! transaction) and filtering the tab shows.

use crate::model::ResultSet;
use std::collections::HashSet;

/// An active query running longer than this (seconds) is highlighted.
pub const LONG_RUNNING_SECS: f64 = 60.0;

/// Every session of the server except the one asking. Times are seconds
/// (as float text) so they parse without a date library; the blockers come
/// as an array literal such as `{12,34}`. Columns are positional: keep
/// [`parse`] in step.
pub const ACTIVITY_SQL: &str = "select pid::text, usename::text, datname::text, application_name::text, client_addr::text, state::text, \
    wait_event_type::text, wait_event::text, backend_type::text, \
    extract(epoch from (now() - query_start))::float8::text, \
    extract(epoch from (now() - xact_start))::float8::text, \
    extract(epoch from (now() - state_change))::float8::text, \
    pg_blocking_pids(pid)::text, query::text \
    from pg_stat_activity where pid <> pg_backend_pid() \
    order by coalesce(query_start, backend_start), pid";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub pid: i64,
    pub user: String,
    pub database: String,
    pub application: String,
    pub client: String,
    /// `active`, `idle`, `idle in transaction`… empty for background workers.
    pub state: String,
    pub wait_type: String,
    pub wait_event: String,
    pub backend_type: String,
    /// Seconds since the current (or last) query started.
    pub query_secs: Option<f64>,
    /// Seconds since the open transaction started.
    pub xact_secs: Option<f64>,
    /// Seconds since the state last changed.
    pub state_secs: Option<f64>,
    /// Sessions this one waits for.
    pub blocked_by: Vec<i64>,
    pub query: String,
}

/// What stands out about a session.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flags {
    pub blocked: bool,
    pub blocking: bool,
    pub long_running: bool,
    pub idle_in_tx: bool,
}

impl Session {
    pub fn is_idle(&self) -> bool {
        // Background workers have no state at all: treat them as idle noise.
        self.state.is_empty() || self.state == "idle"
    }

    pub fn is_active(&self) -> bool {
        self.state == "active"
    }

    pub fn is_idle_in_tx(&self) -> bool {
        self.state.starts_with("idle in transaction")
    }

    /// How long the session has been doing what it does now: the query for
    /// an active one, the wait in the current state otherwise.
    pub fn running_secs(&self) -> Option<f64> {
        if self.is_active() { self.query_secs } else { self.state_secs }
    }

    /// `Wait event type: event`, empty when not waiting.
    pub fn wait(&self) -> String {
        match (self.wait_type.is_empty(), self.wait_event.is_empty()) {
            (true, true) => String::new(),
            (false, true) => self.wait_type.clone(),
            (true, false) => self.wait_event.clone(),
            (false, false) => format!("{}: {}", self.wait_type, self.wait_event),
        }
    }

    /// Whether `needle` (lowercase, trimmed) appears in any shown field.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let fields = [&self.user, &self.database, &self.application, &self.client, &self.state, &self.wait_type, &self.wait_event, &self.backend_type, &self.query];
        self.pid.to_string().contains(needle) || fields.iter().any(|f| f.to_lowercase().contains(needle))
    }
}

/// Flags for each session, in order. A session is blocking when another
/// one lists it in `blocked_by`.
pub fn classify(sessions: &[Session]) -> Vec<Flags> {
    let blockers: HashSet<i64> = sessions.iter().flat_map(|s| s.blocked_by.iter().copied()).collect();
    sessions
        .iter()
        .map(|s| Flags {
            blocked: !s.blocked_by.is_empty(),
            blocking: blockers.contains(&s.pid),
            long_running: s.is_active() && s.query_secs.is_some_and(|t| t > LONG_RUNNING_SECS),
            idle_in_tx: s.is_idle_in_tx(),
        })
        .collect()
}

/// Indexes of the sessions to show: idle ones only with `show_idle` (except
/// those that matter: blocked, blocking or idle in transaction are never
/// idle), and those matching the text filter.
pub fn visible(sessions: &[Session], flags: &[Flags], show_idle: bool, filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    (0..sessions.len()).filter(|&i| (show_idle || !sessions[i].is_idle() || flags[i].blocking) && sessions[i].matches(&needle)).collect()
}

/// Rows of [`ACTIVITY_SQL`] as sessions; rows that don't parse are skipped.
pub fn parse(rs: &ResultSet) -> Vec<Session> {
    rs.rows
        .iter()
        .filter_map(|r| {
            let text = |i: usize| r.get(i).cloned().flatten().unwrap_or_default();
            let secs = |i: usize| r.get(i).cloned().flatten().and_then(|v| v.trim().parse::<f64>().ok());
            Some(Session {
                pid: text(0).trim().parse().ok()?,
                user: text(1),
                database: text(2),
                application: text(3),
                client: text(4),
                state: text(5),
                wait_type: text(6),
                wait_event: text(7),
                backend_type: text(8),
                query_secs: secs(9),
                xact_secs: secs(10),
                state_secs: secs(11),
                blocked_by: parse_pids(&text(12)),
                query: text(13),
            })
        })
        .collect()
}

/// `{12,34}` → `[12, 34]`.
pub fn parse_pids(s: &str) -> Vec<i64> {
    s.trim().trim_start_matches('{').trim_end_matches('}').split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// `45s`, `3m 05s`, `2h 10m`, `1d 4h`; empty for no time.
pub fn format_duration(secs: Option<f64>) -> String {
    let Some(secs) = secs else { return String::new() };
    let s = secs.max(0.0);
    if s < 10.0 {
        return format!("{s:.1}s");
    }
    let t = s as u64;
    match t {
        0..=59 => format!("{t}s"),
        60..=3599 => format!("{}m {:02}s", t / 60, t % 60),
        3600..=86399 => format!("{}h {:02}m", t / 3600, t % 3600 / 60),
        _ => format!("{}d {}h", t / 86400, t % 86400 / 3600),
    }
}

/// `select pg_cancel_backend(pid)` (stop the query) or
/// `pg_terminate_backend` (drop the session). Returns one boolean.
pub fn signal_sql(pid: i64, terminate: bool) -> String {
    format!("select {}({pid})::text", if terminate { "pg_terminate_backend" } else { "pg_cancel_backend" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(pid: i64, state: &str) -> Session {
        Session { pid, state: state.into(), ..Default::default() }
    }

    #[test]
    fn pid_arrays() {
        assert_eq!(parse_pids("{}"), Vec::<i64>::new());
        assert_eq!(parse_pids("{12,34}"), vec![12, 34]);
        assert_eq!(parse_pids(" {7} "), vec![7]);
        assert!(parse_pids("").is_empty());
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(None), "");
        assert_eq!(format_duration(Some(0.25)), "0.2s");
        assert_eq!(format_duration(Some(45.9)), "45s");
        assert_eq!(format_duration(Some(185.0)), "3m 05s");
        assert_eq!(format_duration(Some(7800.0)), "2h 10m");
        assert_eq!(format_duration(Some(100_800.0)), "1d 4h");
        assert_eq!(format_duration(Some(-1.0)), "0.0s");
    }

    #[test]
    fn classifies_blocked_blocking_long_and_idle_in_tx() {
        let mut waiting = session(2, "active");
        waiting.blocked_by = vec![1];
        let mut holder = session(1, "idle in transaction");
        holder.xact_secs = Some(500.0);
        let mut slow = session(3, "active");
        slow.query_secs = Some(61.0);
        let mut quick = session(4, "active");
        quick.query_secs = Some(60.0);
        let mut idle_slow = session(5, "idle");
        idle_slow.query_secs = Some(9999.0);
        let flags = classify(&[holder, waiting, slow, quick, idle_slow]);
        assert_eq!(flags[0], Flags { blocking: true, idle_in_tx: true, ..Default::default() });
        assert_eq!(flags[1], Flags { blocked: true, ..Default::default() });
        assert!(flags[2].long_running);
        assert!(!flags[3].long_running, "exactly a minute is not over a minute");
        assert_eq!(flags[4], Flags::default());
    }

    #[test]
    fn aborted_transactions_count_as_idle_in_tx() {
        assert!(session(1, "idle in transaction (aborted)").is_idle_in_tx());
        assert!(!session(1, "idle").is_idle_in_tx());
    }

    #[test]
    fn filters_idle_and_text() {
        let mut a = session(10, "active");
        a.query = "SELECT * FROM Orders".into();
        a.user = "alice".into();
        let b = session(11, "idle");
        let c = session(12, "");
        let mut d = session(13, "idle");
        d.blocked_by = vec![];
        let sessions = vec![a, b, c, d];
        let mut flags = classify(&sessions);
        assert_eq!(visible(&sessions, &flags, false, ""), vec![0]);
        assert_eq!(visible(&sessions, &flags, true, ""), vec![0, 1, 2, 3]);
        assert_eq!(visible(&sessions, &flags, false, " ORDERS "), vec![0]);
        assert_eq!(visible(&sessions, &flags, true, "alice"), vec![0]);
        assert_eq!(visible(&sessions, &flags, true, "13"), vec![3]);
        // An idle session that blocks others stays in view.
        flags[1].blocking = true;
        assert_eq!(visible(&sessions, &flags, false, ""), vec![0, 1]);
    }

    #[test]
    fn parses_rows() {
        let row = |v: &[Option<&str>]| v.iter().map(|x| x.map(str::to_string)).collect::<Vec<_>>();
        let rs = ResultSet {
            columns: Vec::new(),
            rows: vec![
                row(&[Some("42"), Some("bob"), Some("app"), Some("psql"), Some("10.0.0.1/32"), Some("active"), Some("Lock"), Some("relation"), Some("client backend"), Some("12.5"), Some("20"), Some("12.5"), Some("{7,8}"), Some("select 1")]),
                row(&[Some("x")]),
                row(&[Some("43"), None, None, None, None, None, None, None, Some("autovacuum launcher"), None, None, None, Some("{}"), None]),
            ],
            ..Default::default()
        };
        let s = parse(&rs);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].pid, 42);
        assert_eq!(s[0].blocked_by, vec![7, 8]);
        assert_eq!(s[0].query_secs, Some(12.5));
        assert_eq!(s[0].wait(), "Lock: relation");
        assert_eq!(s[1].query, "");
        assert!(s[1].is_idle());
        assert_eq!(s[1].query_secs, None);
    }

    #[test]
    fn signal_statements() {
        assert_eq!(signal_sql(5, false), "select pg_cancel_backend(5)::text");
        assert_eq!(signal_sql(5, true), "select pg_terminate_backend(5)::text");
    }
}
