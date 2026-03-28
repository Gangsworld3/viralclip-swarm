use std::path::Path;
use std::process::{Command, Stdio};

#[test]
#[ignore]
fn e2e_smoke_run_with_fixture() {
    if std::env::var("RUN_E2E").ok().as_deref() != Some("1") {
        eprintln!("RUN_E2E=1 not set; skipping");
        return;
    }

    let fixture = Path::new("tests/fixtures/sample.mp4");
    assert!(
        fixture.is_file(),
        "missing fixture at {}",
        fixture.display()
    );

    let exe = std::env::var("CARGO_BIN_EXE_viralclip-swarm").expect("binary path");
    let status = Command::new(exe)
        .arg("--input")
        .arg(fixture)
        .arg("--num-clips")
        .arg("1")
        .arg("--min-duration")
        .arg("2")
        .arg("--output-dir")
        .arg("output/e2e_smoke")
        .arg("--csv-format")
        .arg("json")
        .arg("--csv-path")
        .arg("output/e2e_smoke/benchmark.json")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run binary");

    assert!(status.success(), "e2e run failed");
}
