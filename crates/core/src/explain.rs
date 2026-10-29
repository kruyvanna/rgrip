//! EXPLAIN plans: builds the statement that asks for one and turns the JSON
//! Postgres answers with into a tree with self time, share and estimate misses.

use crate::model::QueryOutput;
use crate::sql::split_statements;
use serde_json::Value;

/// A node whose actual rows differ from the estimate by at least this factor is flagged.
pub const MISESTIMATE_FACTOR: f64 = 10.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buffers {
    pub shared_hit: i64,
    pub shared_read: i64,
    pub shared_dirtied: i64,
    pub shared_written: i64,
    pub temp_read: i64,
    pub temp_written: i64,
}

/// How far the actual rows per loop are from the estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Miss {
    /// Always >= 1.
    pub factor: f64,
    /// Postgres expected fewer rows than it got.
    pub under: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PlanNode {
    pub node_type: String,
    /// What a person reads: "Index Scan using users_pkey on users u".
    pub label: String,
    pub startup_cost: f64,
    pub total_cost: f64,
    /// Estimated rows per loop.
    pub plan_rows: f64,
    pub plan_width: i64,
    /// Present with ANALYZE; `None` for a node that never ran under one.
    pub actual: Option<Actual>,
    /// Cost of this node alone (total minus its children's).
    pub self_cost: f64,
    /// Share of the plan's weight: self time over execution time with
    /// ANALYZE, self cost over total cost without. 0..=1.
    pub share: f64,
    pub miss: Option<Miss>,
    pub buffers: Option<Buffers>,
    /// Other properties worth showing (conditions, sort keys, …), in display order.
    pub details: Vec<(String, String)>,
    pub children: Vec<PlanNode>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Actual {
    /// Rows per loop, as Postgres reports them.
    pub rows: f64,
    pub loops: f64,
    /// Time per loop until all rows were returned, in ms.
    pub time_ms: f64,
    /// `time_ms` over all loops.
    pub total_ms: f64,
    /// `total_ms` minus the children's, never below zero.
    pub self_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub root: PlanNode,
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    /// The JSON as returned, for copying.
    pub raw: String,
}

impl Plan {
    pub fn node_count(&self) -> usize {
        fn count(n: &PlanNode) -> usize {
            1 + n.children.iter().map(count).sum::<usize>()
        }
        count(&self.root)
    }
}

/// The EXPLAIN statement for `stmt` (one statement; a trailing `;` is dropped).
pub fn explain_sql(stmt: &str, analyze: bool) -> String {
    let stmt = stmt.trim().trim_end_matches(';').trim_end();
    if analyze {
        format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {stmt}")
    } else {
        format!("EXPLAIN (FORMAT JSON, VERBOSE, COSTS) {stmt}")
    }
}

/// `explain_sql` inside a transaction that is rolled back, so ANALYZE of a
/// write changes nothing. One simple-query message, so nothing else on the
/// connection can slip in between BEGIN and ROLLBACK.
pub fn rolled_back_sql(explain: &str) -> String {
    format!("BEGIN; {explain}; ROLLBACK;")
}

/// Whether `sql` is exactly one statement (comments aside).
pub fn is_single_statement(sql: &str) -> bool {
    split_statements(sql).iter().filter(|s| !sql[s.trimmed.clone()].trim().is_empty()).count() == 1
}

/// The plan in a query's output when it is a single `QUERY PLAN` cell of JSON
/// (what `EXPLAIN (FORMAT JSON)` returns), else `None`.
pub fn plan_from_output(out: &QueryOutput) -> Option<Plan> {
    let rs = out.statements.iter().rev().find_map(|s| s.result.as_ref())?;
    if rs.columns.len() != 1 || !rs.columns[0].eq_ignore_ascii_case("QUERY PLAN") || rs.rows.len() != 1 {
        return None;
    }
    parse(rs.rows[0][0].as_deref()?).ok()
}

/// Parse the JSON of `EXPLAIN (FORMAT JSON)`.
pub fn parse(json: &str) -> Result<Plan, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Not a JSON plan: {e}"))?;
    let top = match &v {
        Value::Array(a) => a.first(),
        other => Some(other),
    };
    let Some(top) = top.filter(|t| t.get("Plan").is_some()) else { return Err("Not a plan: no \"Plan\" object".into()) };
    let mut root = node(&top["Plan"]);
    let analyzed = root.actual.is_some() || top.get("Execution Time").is_some();
    fill_derived(&mut root);
    let execution_ms = top.get("Execution Time").and_then(Value::as_f64);
    let weight = if analyzed { execution_ms.or(root.actual.as_ref().map(|a| a.total_ms)).unwrap_or(0.0) } else { root.total_cost };
    shares(&mut root, analyzed, weight);
    Ok(Plan { root, analyzed, planning_ms: top.get("Planning Time").and_then(Value::as_f64), execution_ms, raw: json.to_string() })
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn node(v: &Value) -> PlanNode {
    let actual = num(v, "Actual Total Time").map(|t| {
        let loops = num(v, "Actual Loops").unwrap_or(1.0);
        Actual { rows: num(v, "Actual Rows").unwrap_or(0.0), loops, time_ms: t, total_ms: t * loops, self_ms: 0.0 }
    });
    let buffers = v.get("Shared Hit Blocks").or(v.get("Shared Read Blocks")).or(v.get("Temp Read Blocks")).map(|_| {
        let n = |k| v.get(k).and_then(Value::as_i64).unwrap_or(0);
        Buffers {
            shared_hit: n("Shared Hit Blocks"),
            shared_read: n("Shared Read Blocks"),
            shared_dirtied: n("Shared Dirtied Blocks"),
            shared_written: n("Shared Written Blocks"),
            temp_read: n("Temp Read Blocks"),
            temp_written: n("Temp Written Blocks"),
        }
    });
    let mut n = PlanNode {
        node_type: text(v, "Node Type").unwrap_or("?").to_string(),
        label: label(v),
        startup_cost: num(v, "Startup Cost").unwrap_or(0.0),
        total_cost: num(v, "Total Cost").unwrap_or(0.0),
        plan_rows: num(v, "Plan Rows").unwrap_or(0.0),
        plan_width: v.get("Plan Width").and_then(Value::as_i64).unwrap_or(0),
        actual,
        buffers,
        details: details(v),
        ..Default::default()
    };
    if let Some(kids) = v.get("Plans").and_then(Value::as_array) {
        n.children = kids.iter().map(node).collect();
    }
    n
}

/// "Index Scan using users_pkey on users u", "Hash Left Join", "HashAggregate".
fn label(v: &Value) -> String {
    let kind = text(v, "Node Type").unwrap_or("?");
    let mut s = match (kind, text(v, "Join Type"), text(v, "Strategy")) {
        ("Hash Join" | "Merge Join", Some(j), _) if j != "Inner" => kind.replace("Join", &format!("{j} Join")),
        ("Nested Loop", Some(j), _) if j != "Inner" => format!("Nested Loop {j} Join"),
        ("Aggregate", _, Some("Hashed")) => "HashAggregate".into(),
        ("Aggregate", _, Some("Sorted")) => "GroupAggregate".into(),
        _ => kind.to_string(),
    };
    if let Some(mode @ ("Partial" | "Finalize")) = text(v, "Partial Mode").filter(|_| kind == "Aggregate") {
        s = format!("{mode} {s}");
    }
    if let Some(i) = text(v, "Index Name") {
        // A bitmap index scan reads one index; the other scans go through it to a table.
        if kind != "Bitmap Index Scan" {
            s += " using ";
        } else {
            s += " on ";
        }
        s += i;
    }
    let target = text(v, "Relation Name").map(|r| match text(v, "Schema") {
        Some(sc) if sc != "public" => format!("{sc}.{r}"),
        _ => r.to_string(),
    });
    let target = target.or_else(|| text(v, "CTE Name").map(String::from)).or_else(|| text(v, "Function Name").map(String::from));
    if let Some(t) = target {
        s += &format!(" on {t}");
        match text(v, "Alias") {
            Some(a) if a != t.rsplit('.').next().unwrap_or(&t) => s += &format!(" {a}"),
            _ => {}
        }
    }
    if let Some(sub) = text(v, "Subplan Name") {
        s += &format!("  [{sub}]");
    }
    s
}

const DETAIL_KEYS: &[&str] = &[
    "Output", "Index Cond", "Recheck Cond", "Hash Cond", "Merge Cond", "Join Filter", "Filter", "Rows Removed by Filter", "Rows Removed by Join Filter", "Rows Removed by Index Recheck",
    "Sort Key", "Group Key", "Sort Method", "Sort Space Used", "Sort Space Type", "Heap Fetches", "Exact Heap Blocks", "Lossy Heap Blocks", "Hash Buckets", "Hash Batches",
    "Peak Memory Usage", "Workers Planned", "Workers Launched", "Parallel Aware",
];

fn details(v: &Value) -> Vec<(String, String)> {
    DETAIL_KEYS
        .iter()
        .filter_map(|k| {
            let s = match v.get(*k)? {
                Value::String(s) => s.clone(),
                Value::Array(a) => a.iter().map(|x| x.as_str().map_or_else(|| x.to_string(), String::from)).collect::<Vec<_>>().join(", "),
                Value::Bool(false) => return None,
                other => other.to_string(),
            };
            Some((k.to_string(), s))
        })
        .collect()
}

/// Self time and cost (node minus children) and the estimate miss.
fn fill_derived(n: &mut PlanNode) {
    for c in &mut n.children {
        fill_derived(c);
    }
    n.self_cost = (n.total_cost - n.children.iter().map(|c| c.total_cost).sum::<f64>()).max(0.0);
    let kids_ms: f64 = n.children.iter().filter_map(|c| c.actual.as_ref()).map(|a| a.total_ms).sum();
    if let Some(a) = &mut n.actual {
        a.self_ms = (a.total_ms - kids_ms).max(0.0);
        // A node that never ran (loops = 0) has nothing to compare.
        if a.loops > 0.0 {
            let (act, est) = (a.rows, n.plan_rows);
            let (hi, lo) = (act.max(est), act.min(est).max(1.0));
            let factor = hi.max(1.0) / lo;
            n.miss = (factor >= MISESTIMATE_FACTOR).then_some(Miss { factor, under: act > est });
        }
    }
}

fn shares(n: &mut PlanNode, analyzed: bool, weight: f64) {
    let own = if analyzed { n.actual.as_ref().map_or(0.0, |a| a.self_ms) } else { n.self_cost };
    n.share = if weight > 0.0 { (own / weight).clamp(0.0, 1.0) } else { 0.0 };
    for c in &mut n.children {
        shares(c, analyzed, weight);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = r#"[{"Plan": {"Node Type": "Nested Loop", "Parallel Aware": false, "Join Type": "Inner", "Startup Cost": 0.29, "Total Cost": 25.71, "Plan Rows": 5, "Plan Width": 72,
      "Output": ["u.id", "o.total"],
      "Plans": [
        {"Node Type": "Seq Scan", "Parent Relationship": "Outer", "Relation Name": "users", "Schema": "public", "Alias": "u", "Startup Cost": 0.0, "Total Cost": 10.5, "Plan Rows": 5, "Plan Width": 36, "Filter": "(active)"},
        {"Node Type": "Index Scan", "Parent Relationship": "Inner", "Index Name": "orders_user_id_idx", "Relation Name": "orders", "Schema": "public", "Alias": "o", "Startup Cost": 0.29, "Total Cost": 3.0, "Plan Rows": 1, "Plan Width": 36, "Index Cond": "(user_id = u.id)"}
      ]}}]"#;

    const ANALYZED: &str = r#"[{"Plan": {"Node Type": "Hash Join", "Join Type": "Left", "Parallel Aware": false, "Startup Cost": 1.1, "Total Cost": 500.0, "Plan Rows": 10, "Plan Width": 8,
      "Actual Startup Time": 0.5, "Actual Total Time": 100.0, "Actual Rows": 5000, "Actual Loops": 1, "Hash Cond": "(a.id = b.a_id)",
      "Shared Hit Blocks": 120, "Shared Read Blocks": 30, "Shared Dirtied Blocks": 0, "Shared Written Blocks": 0, "Temp Read Blocks": 0, "Temp Written Blocks": 0,
      "Plans": [
        {"Node Type": "Seq Scan", "Relation Name": "a", "Schema": "sales", "Alias": "a", "Startup Cost": 0.0, "Total Cost": 400.0, "Plan Rows": 10, "Plan Width": 4,
         "Actual Startup Time": 0.01, "Actual Total Time": 60.0, "Actual Rows": 5000, "Actual Loops": 1, "Rows Removed by Filter": 12},
        {"Node Type": "Index Only Scan", "Index Name": "b_pkey", "Relation Name": "b", "Schema": "public", "Alias": "bb", "Startup Cost": 0.1, "Total Cost": 8.0, "Plan Rows": 100, "Plan Width": 4,
         "Actual Startup Time": 0.001, "Actual Total Time": 0.004, "Actual Rows": 1, "Actual Loops": 5000, "Heap Fetches": 0}
      ]},
     "Planning Time": 0.25, "Triggers": [], "Execution Time": 101.5}]"#;

    #[test]
    fn plain_plan_tree_and_cost_share() {
        let p = parse(PLAIN).unwrap();
        assert!(!p.analyzed);
        assert_eq!(p.node_count(), 3);
        assert_eq!(p.root.label, "Nested Loop");
        assert_eq!(p.root.children[0].label, "Seq Scan on users u");
        assert_eq!(p.root.children[1].label, "Index Scan using orders_user_id_idx on orders o");
        assert!(p.root.actual.is_none() && p.planning_ms.is_none());
        // Parent cost 25.71 minus 10.5 and 3.0 is the join's own.
        assert!((p.root.self_cost - 12.21).abs() < 1e-9);
        assert!((p.root.children[0].share - 10.5 / 25.71).abs() < 1e-9);
        assert_eq!(p.root.details, vec![("Output".to_string(), "u.id, o.total".to_string())]);
        assert_eq!(p.root.children[0].details, vec![("Filter".to_string(), "(active)".to_string())]);
    }

    #[test]
    fn analyzed_plan_self_time_loops_and_misses() {
        let p = parse(ANALYZED).unwrap();
        assert!(p.analyzed);
        assert_eq!((p.planning_ms, p.execution_ms), (Some(0.25), Some(101.5)));
        let r = &p.root;
        assert_eq!(r.label, "Hash Left Join");
        let (scan, idx) = (&r.children[0], &r.children[1]);
        assert_eq!(scan.label, "Seq Scan on sales.a");
        assert_eq!(idx.label, "Index Only Scan using b_pkey on b bb");
        // 0.004 ms x 5000 loops = 20 ms for the inner side.
        let ia = idx.actual.as_ref().unwrap();
        assert!((ia.total_ms - 20.0).abs() < 1e-9);
        // Join: 100 - (60 + 20).
        assert!((r.actual.as_ref().unwrap().self_ms - 20.0).abs() < 1e-9);
        assert!((scan.share - 60.0 / 101.5).abs() < 1e-9);
        // 5000 rows where 10 were expected is a 500x underestimate; 1 row where 100 were is a 100x overestimate.
        let m = r.miss.unwrap();
        assert!(m.under && (m.factor - 500.0).abs() < 1e-9);
        let m = idx.miss.unwrap();
        assert!(!m.under && (m.factor - 100.0).abs() < 1e-9);
        assert_eq!(r.buffers.as_ref().unwrap().shared_hit, 120);
        assert_eq!(r.buffers.as_ref().unwrap().shared_read, 30);
        assert!(scan.buffers.is_none());
        assert_eq!(scan.details, vec![("Rows Removed by Filter".to_string(), "12".to_string())]);
    }

    #[test]
    fn small_differences_are_not_misses() {
        let json = r#"[{"Plan": {"Node Type": "Result", "Startup Cost": 0, "Total Cost": 0.01, "Plan Rows": 1, "Plan Width": 4, "Actual Total Time": 0.01, "Actual Rows": 0, "Actual Loops": 1}}]"#;
        let p = parse(json).unwrap();
        assert!(p.root.miss.is_none());
    }

    #[test]
    fn never_executed_node_has_no_miss_and_no_time() {
        let json = r#"[{"Plan": {"Node Type": "Seq Scan", "Relation Name": "t", "Alias": "t", "Startup Cost": 0, "Total Cost": 5, "Plan Rows": 1000, "Plan Width": 4, "Actual Total Time": 0.0, "Actual Rows": 0, "Actual Loops": 0}, "Execution Time": 0.1}]"#;
        let p = parse(json).unwrap();
        assert!(p.root.miss.is_none());
        assert_eq!(p.root.actual.as_ref().unwrap().total_ms, 0.0);
    }

    #[test]
    fn rejects_non_plans() {
        assert!(parse("not json").is_err());
        assert!(parse("[]").is_err());
        assert!(parse(r#"[{"a": 1}]"#).is_err());
    }

    #[test]
    fn statements_for_explain() {
        assert_eq!(explain_sql("select 1;  ", false), "EXPLAIN (FORMAT JSON, VERBOSE, COSTS) select 1");
        assert_eq!(explain_sql("delete from t;", true), "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) delete from t");
        assert_eq!(rolled_back_sql("EXPLAIN x"), "BEGIN; EXPLAIN x; ROLLBACK;");
        assert!(is_single_statement("select 1;"));
        assert!(is_single_statement("-- c\nselect 1"));
        assert!(!is_single_statement("select 1; select 2"));
        assert!(!is_single_statement("  "));
    }

    #[test]
    fn plan_from_a_query_output() {
        use crate::model::{ResultSet, StatementResult};
        let out = |cols: &[&str], cell: &str| QueryOutput {
            statements: vec![StatementResult {
                result: Some(ResultSet { columns: cols.iter().map(|s| s.to_string()).collect(), rows: vec![vec![Some(cell.to_string())]], ..Default::default() }),
                affected: 1,
            }],
            elapsed: Default::default(),
        };
        assert!(plan_from_output(&out(&["QUERY PLAN"], PLAIN)).is_some());
        assert!(plan_from_output(&out(&["x"], PLAIN)).is_none());
        assert!(plan_from_output(&out(&["QUERY PLAN"], "Seq Scan on t")).is_none());
    }
}
