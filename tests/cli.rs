//! End-to-end checks against the built binary and the system `ps`.

use std::process::{Command, Output};

fn pss(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pss"))
        .args(args)
        .env_remove("NO_COLOR")
        .output()
        .expect("run pss")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn fields(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

#[test]
fn agrees_with_system_ps_on_this_process() {
    let me = std::process::id().to_string();
    let keywords = "pid=,ppid=,pgid=,user=,uid=,tt=,nice=,ucomm=";
    let ours = pss(&["-o", keywords, "-p", &me]);
    let theirs = Command::new("/bin/ps")
        .args(["-o", keywords, "-p", &me])
        .output()
        .unwrap();
    assert!(ours.status.success());
    assert_eq!(
        fields(&stdout(&ours)),
        fields(&String::from_utf8_lossy(&theirs.stdout))
    );
}

#[test]
fn piped_output_is_plain_and_unpaged() {
    let o = pss(&["aux"]);
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(!out.contains('\x1b'), "no ANSI escapes when piped");
    assert!(out.starts_with("USER"));
    assert!(out.lines().count() > 10);
}

#[test]
fn color_always_forces_escapes() {
    let out = stdout(&pss(&["-p", "1", "--color=always"]));
    assert!(out.contains("\x1b["));
}

#[test]
fn pattern_excludes_pss_and_sets_exit_code() {
    let o = pss(&["-o", "pid=,command=", "--", "no-such-process-pss-test"]);
    assert_eq!(o.status.code(), Some(1));
    // `--` must not let pss match its own command line.
    assert!(stdout(&o).is_empty());

    let o = pss(&["launchd"]);
    assert!(o.status.success());
    assert!(fields(&stdout(&o)).contains(&"1".to_string()));
}

#[test]
fn bad_input_fails_with_message() {
    let o = pss(&["-o", "nope"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("nope: keyword not found"));

    let o = pss(&["-u", "no-such-user-pss"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn help_version_and_keyword_list() {
    assert!(stdout(&pss(&["--help"])).contains("Usage: pss"));
    assert!(stdout(&pss(&["-V"])).starts_with("pss "));
    assert!(stdout(&pss(&["-L"])).contains("%cpu"));
}

#[test]
fn sort_descending_by_pid() {
    let out = stdout(&pss(&["-A", "-o", "pid=", "--sort", "-pid"]));
    let pids: Vec<i64> = out.split_whitespace().map(|p| p.parse().unwrap()).collect();
    assert!(pids.windows(2).all(|w| w[0] > w[1]));
}
