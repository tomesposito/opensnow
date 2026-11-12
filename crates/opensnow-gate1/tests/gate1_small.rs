//! Small-scale Gate 1 harness test: divisor 200,000 gives 10,000
//! sales_fact rows and 30,000 clickstream rows. Asserts the yardstick
//! checks pass, no runnable statement fails, and every statement runs
//! (the MERGE and UPDATE-FROM statements included, since issue #18).
use opensnow_gate1::run;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gate1_harness_small_scale() {
    let (report, _harness, _data) = run(200_000).await.unwrap();
    assert_eq!(report.global_check, "pass", "global yardstick");
    assert!(report.write_check.starts_with("pass"), "write path: {}", report.write_check);
    assert!(report.burst.starts_with("30/30"), "burst: {}", report.burst);
    assert!(report.total_parquet_bytes > 0);
    let mut could_not = Vec::new();
    for s in &report.statements {
        assert_ne!(s.check.starts_with("fail"), true, "{} check: {}", s.id, s.check);
        assert_ne!(s.status, "failed", "{} failed: {}", s.id, s.check);
        if s.status == "could_not_run" { could_not.push(s.id.as_str()); }
        if s.status == "timed" {
            assert!(s.median_seconds.is_some(), "{} untimed", s.id);
            if s.check != "none" { assert_eq!(s.check, "pass", "{} check", s.id); }
        }
    }
    assert!(could_not.is_empty(), "statements that could not run: {could_not:?}");
    assert_eq!(report.statements.len(), 33);
}
