use opensnow_control::{guard_sql, Access, ControlError};

const NS: &str = "t0123456789abcdef";

fn denied(sql: &str) -> bool {
    matches!(guard_sql(NS, sql), Err(ControlError::AccessDenied(_)))
}

#[test]
fn quoted_table_references_are_rejected() {
    // Audit reproduction for issue #23: a fully quoted three-part
    // reference to another tenant's namespace passed the guard
    // unchanged and executed against the shared catalog.
    assert!(denied(r#"SELECT id FROM "ice"."tdeadbeefdeadbeef"."events""#));
    // The same reference to the caller's own namespace was also
    // executed verbatim; quoted names are never the guard's output,
    // so they are rejected in every position.
    assert!(denied(r#"SELECT id FROM "ice"."t0123456789abcdef"."events""#));
    assert!(denied(r#"SELECT id FROM "events""#));
    assert!(denied(r#"SELECT id FROM ice."t0123456789abcdef".events"#));
    // A quoted branch of a set operation is a table reference too.
    assert!(denied(
        r#"SELECT id FROM events UNION ALL SELECT id FROM "ice"."tdeadbeefdeadbeef"."events""#
    ));
    assert!(denied(
        r#"SELECT id FROM events JOIN "events" AS other ON events.id = other.id"#
    ));
    // Write targets are covered as well as reads.
    assert!(denied(r#"INSERT INTO "events" VALUES (1)"#));
    assert!(denied(r#"CREATE TABLE "events" (id BIGINT NOT NULL)"#));
    assert!(denied(r#"UPDATE "events" SET id = 1"#));
    assert!(denied(r#"DELETE FROM "events""#));
    assert!(denied(r#"DROP TABLE "events""#));
    // A double quote anywhere in the statement fails closed, even
    // inside a string literal: the guard never emits one, and a
    // lexical scan cannot tell identifier quotes from literal ones.
    assert!(denied(r#"SELECT id FROM events WHERE id = "1""#));
}

#[test]
fn plain_table_references_still_qualify() {
    let (rewritten, access) = guard_sql(NS, "SELECT id FROM events").unwrap();
    assert_eq!(rewritten, format!("SELECT id FROM ice.{NS}.events"));
    assert!(matches!(access, Access::Read));

    let (rewritten, _) = guard_sql(NS, &format!("SELECT id FROM ice.{NS}.events")).unwrap();
    assert_eq!(rewritten, format!("SELECT id FROM ice.{NS}.events"));

    let (rewritten, _) = guard_sql(NS, &format!("SELECT id FROM {NS}.events")).unwrap();
    assert_eq!(rewritten, format!("SELECT id FROM ice.{NS}.events"));

    let (rewritten, access) = guard_sql(NS, "INSERT INTO events VALUES (1)").unwrap();
    assert_eq!(rewritten, format!("INSERT INTO ice.{NS}.events VALUES (1)"));
    assert!(matches!(access, Access::Write));

    // The unquoted cross-tenant reference was already denied; it
    // stays denied after the quoted fix.
    assert!(denied("SELECT id FROM ice.tdeadbeefdeadbeef.events"));
    assert!(denied("SELECT id FROM tdeadbeefdeadbeef.events"));
}

#[test]
fn comma_join_references_are_denied_or_qualified() {
    // Audit reproduction for issue #22: the lexical qualifier captured
    // only the token after FROM or JOIN, so every later element of a
    // comma separated FROM list passed through unchecked.
    assert!(denied("SELECT id FROM events, ice.tdeadbeefdeadbeef.events"));
    assert!(denied("SELECT id FROM events, tdeadbeefdeadbeef.events"));
    // The victim in the first position was already denied and stays
    // denied when an own table follows it.
    assert!(denied("SELECT id FROM ice.tdeadbeefdeadbeef.events, events"));
    // The victim as the last of three elements is denied too.
    assert!(denied("SELECT id FROM events, other_events, ice.tdeadbeefdeadbeef.events"));
    // Comma lists in a UNION branch and inside subqueries are covered.
    assert!(denied(
        "SELECT id FROM events UNION ALL SELECT id FROM other_events, ice.tdeadbeefdeadbeef.events"
    ));
    assert!(denied(
        "SELECT id FROM events WHERE id IN (SELECT id FROM other_events, ice.tdeadbeefdeadbeef.events)"
    ));
    assert!(denied(
        "SELECT id FROM (SELECT id FROM other_events, ice.tdeadbeefdeadbeef.events) AS sub"
    ));
    // The INSERT INTO ... SELECT exfiltration shape is denied: the
    // comma joined source is checked, not just the insert target.
    assert!(denied("INSERT INTO events SELECT id FROM other_events, ice.tdeadbeefdeadbeef.events"));
    assert!(denied("INSERT INTO events SELECT id FROM other_events, tdeadbeefdeadbeef.events"));

    // A legitimate comma join of two own tables qualifies both names.
    let (rewritten, access) = guard_sql(NS, "SELECT id FROM events, other_events").unwrap();
    assert_eq!(rewritten, format!("SELECT id FROM ice.{NS}.events, ice.{NS}.other_events"));
    assert!(matches!(access, Access::Read));

    // A bare own table mixed with an explicit own three part name
    // passes, with both rendered in the guard's own form.
    let (rewritten, _) = guard_sql(NS, &format!("SELECT id FROM events, ice.{NS}.other_events")).unwrap();
    assert_eq!(rewritten, format!("SELECT id FROM ice.{NS}.events, ice.{NS}.other_events"));

    // A legitimate INSERT INTO ... SELECT with a comma source
    // qualifies the target and both source tables.
    let (rewritten, access) =
        guard_sql(NS, "INSERT INTO events SELECT id FROM other_events, more_events").unwrap();
    assert_eq!(
        rewritten,
        format!("INSERT INTO ice.{NS}.events SELECT id FROM ice.{NS}.other_events, ice.{NS}.more_events")
    );
    assert!(matches!(access, Access::Write));

    // A plain JOIN still qualifies both sides.
    let (rewritten, _) =
        guard_sql(NS, "SELECT id FROM events JOIN other_events ON events.id = other_events.id").unwrap();
    assert_eq!(
        rewritten,
        format!("SELECT id FROM ice.{NS}.events JOIN ice.{NS}.other_events ON events.id = other_events.id")
    );
}

#[test]
fn cte_names_are_not_qualified_as_tables() {
    // A CTE reference names the query's own derived result. Qualifying
    // it would point it at a stored table that does not exist. Tables
    // inside the CTE body and beside the reference are still bound to
    // the tenant namespace.
    let (rewritten, access) =
        guard_sql(NS, "WITH recent AS (SELECT id FROM events) SELECT id FROM recent, other_events").unwrap();
    assert_eq!(
        rewritten,
        format!(
            "WITH recent AS (SELECT id FROM ice.{NS}.events) SELECT id FROM recent, ice.{NS}.other_events"
        )
    );
    assert!(matches!(access, Access::Read));
    // A cross-tenant table beside a CTE reference is still denied.
    assert!(denied(
        "WITH recent AS (SELECT id FROM events) SELECT id FROM recent, ice.tdeadbeefdeadbeef.events"
    ));
}
