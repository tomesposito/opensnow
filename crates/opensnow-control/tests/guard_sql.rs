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
