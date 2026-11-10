//! Gate 1 evaluation binary. Usage:
//!   gate1 [--divisor N] [--json PATH]
//! Default divisor 1000 (sales_fact 2,000,000 rows). Prints a markdown
//! summary to stdout and, with --json, the full report as JSON.

use opensnow_gate1::run;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut divisor: i64 = 1000;
    let mut json_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--divisor" => { divisor = args[i + 1].parse().expect("divisor"); i += 2; }
            "--json" => { json_path = Some(args[i + 1].clone()); i += 2; }
            other => { eprintln!("unknown arg {other}"); std::process::exit(2); }
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4).enable_all().build().unwrap();
    let (report, _harness, _data) = runtime.block_on(run(divisor)).unwrap_or_else(|e| {
        eprintln!("gate1 run failed: {e}");
        std::process::exit(1);
    });
    println!("# Gate 1 harness run, divisor {} (1:{})", report.divisor, report.divisor);
    println!("generation {:.1}s, load {:.1}s", report.gen_seconds, report.load_seconds);
    println!("cold starts (s): {:?}", report.cold_start_seconds);
    println!("global yardstick check: {}", report.global_check);
    println!();
    println!("| Stmt | Status | Median s | Rows | Check |");
    println!("|---|---|---|---|---|");
    for s in &report.statements {
        println!("| {} | {} | {} | {} | {} |", s.id,
            s.status,
            s.median_seconds.map(|v| format!("{v:.3}")).unwrap_or_default(),
            s.rows.map(|v| v.to_string()).unwrap_or_default(),
            s.check);
    }
    println!();
    println!("write path: {}", report.write_check);
    println!("burst: {}", report.burst);
    println!("parquet bytes total: {}", report.total_parquet_bytes);
    for (table, count, bytes) in &report.files {
        println!("files {table}: {count} files, {bytes} bytes");
    }
    if let Some(path) = json_path {
        let mut out = String::from("{\n");
        out += &format!("\"divisor\": {},\n\"gen_seconds\": {},\n\"load_seconds\": {},\n",
            report.divisor, report.gen_seconds, report.load_seconds);
        out += &format!("\"cold_start_seconds\": {:?},\n", report.cold_start_seconds);
        out += &format!("\"global_check\": {:?},\n\"write_check\": {:?},\n\"burst\": {:?},\n",
            report.global_check, report.write_check, report.burst);
        out += &format!("\"total_parquet_bytes\": {},\n\"files\": {:?},\n\"statements\": [\n",
            report.total_parquet_bytes, report.files);
        for (idx, s) in report.statements.iter().enumerate() {
            out += &format!("{{\"id\": {:?}, \"workload\": {:?}, \"ledger\": {:?}, \"status\": {:?}, \"median_seconds\": {:?}, \"runs_seconds\": {:?}, \"rows\": {:?}, \"check\": {:?}}}{}\n",
                s.id, s.workload, s.ledger, s.status, s.median_seconds, s.runs_seconds, s.rows, s.check,
                if idx + 1 == report.statements.len() { "" } else { "," });
        }
        out += "]\n}\n";
        std::fs::write(&path, out).unwrap();
        println!("wrote {path}");
    }
}
