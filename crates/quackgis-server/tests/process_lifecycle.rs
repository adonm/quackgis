// SPDX-License-Identifier: Apache-2.0
//! OS-process lifecycle gates for failures and Kubernetes-style termination.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File, FileTimes};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime};

use tokio_postgres::{Client, NoTls};

const BARRIER_ENV: &str = "QUACKGIS_TEST_NATIVE_MUTATION_BARRIER";
const BARRIER_READY_ENV: &str = "QUACKGIS_TEST_NATIVE_MUTATION_BARRIER_READY";
const BARRIER_RELEASE_ENV: &str = "QUACKGIS_TEST_NATIVE_MUTATION_BARRIER_RELEASE";
const MUTATION_TABLE: &str = "native_process_barrier_points";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

static MUTATION_PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn server_command(port: u16, tmp: &tempfile::TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_quackgis-server"));
    command
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--catalog-path")
        .arg(tmp.path().join("quackgis.db"))
        .arg("--data-path")
        .arg(tmp.path().join("data"))
        .arg("--log")
        .arg("warn")
        .env("QUACKGIS_TARGET_PARTITIONS", "1")
        .env_remove(BARRIER_ENV)
        .env_remove(BARRIER_READY_ENV)
        .env_remove(BARRIER_RELEASE_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    command
}

struct ChildGuard {
    child: Child,
    log_path: PathBuf,
}

impl ChildGuard {
    fn spawn(mut command: Command, log_path: PathBuf) -> Self {
        let log = File::create(&log_path).expect("create child log");
        command.stderr(Stdio::from(log));
        let child = command.spawn().expect("spawn server process");
        Self { child, log_path }
    }

    fn diagnostics(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_else(|err| {
            format!(
                "<could not read child log {}: {err}>",
                self.log_path.display()
            )
        })
    }

    fn assert_running(&mut self, context: &str) {
        if let Some(status) = self.child.try_wait().expect("poll server process") {
            panic!(
                "server exited during {context}: {status}\n{}",
                self.diagnostics()
            );
        }
    }

    async fn wait_until_listening(&mut self, port: u16) {
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return;
            }
            self.assert_running("startup");
            assert!(
                Instant::now() < deadline,
                "server did not listen in time\n{}",
                self.diagnostics()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_exit(&mut self, timeout: Duration, context: &str) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("poll server process") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit during {context}\n{}",
                self.diagnostics()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn signal_and_wait(&mut self, signal: libc::c_int, context: &str) -> ExitStatus {
        // SAFETY: child.id() identifies the live subprocess owned by this guard.
        let signal_result = unsafe { libc::kill(self.child.id() as libc::pid_t, signal) };
        assert_eq!(
            signal_result,
            0,
            "send signal {signal} during {context}\n{}",
            self.diagnostics()
        );
        self.wait_for_exit(PROCESS_TIMEOUT, context).await
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            // SAFETY: the child is still live and is reaped immediately below.
            let _ = unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGKILL) };
            let _ = self.child.wait();
        }
    }
}

fn reserve_port() -> u16 {
    let reservation = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
    reservation.local_addr().expect("listener address").port()
}

#[test]
fn bind_failure_exits_nonzero() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
    let port = listener.local_addr().expect("listener address").port();
    let tmp = tempfile::TempDir::new().expect("tempdir");

    let status = server_command(port, &tmp)
        .stderr(Stdio::null())
        .status()
        .expect("run server against occupied port");

    assert!(
        !status.success(),
        "bind failure must propagate to process exit"
    );
}

#[test]
fn duckdb_server_backend_requires_feature_in_default_binary() {
    let port = reserve_port();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let output = server_command(port, &tmp)
        .arg("--engine-backend")
        .arg("duckdb")
        .stderr(Stdio::piped())
        .output()
        .expect("run DuckDB backend with default-feature binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a binary built with --features duckdb-adbc"),
        "unexpected stderr: {stderr}"
    );
    assert!(!tmp.path().join("quackgis.db").exists());
    assert!(!tmp.path().join("data").exists());
}

#[test]
fn invalid_configured_tls_exits_instead_of_serving_plaintext() {
    let port = reserve_port();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let certificate = tmp.path().join("invalid-cert.pem");
    let key = tmp.path().join("invalid-key.pem");
    fs::write(&certificate, "not a certificate").expect("write invalid certificate");
    fs::write(&key, "not a private key").expect("write invalid key");

    let output = server_command(port, &tmp)
        .arg("--tls-cert")
        .arg(&certificate)
        .arg("--tls-key")
        .arg(&key)
        .stderr(Stdio::piped())
        .output()
        .expect("run server with invalid TLS material");

    assert!(!output.status.success());
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "server must not remain available over plaintext after TLS setup fails"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to configure requested TLS"),
        "unexpected stderr: {stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_stops_server_cleanly_and_catalog_reopens() {
    let port = reserve_port();
    let tmp = tempfile::TempDir::new().expect("tempdir");

    let mut first = ChildGuard::spawn(server_command(port, &tmp), tmp.path().join("first.log"));
    first.wait_until_listening(port).await;
    assert!(
        first
            .signal_and_wait(libc::SIGTERM, "first SIGTERM")
            .await
            .success(),
        "handled SIGTERM should exit successfully\n{}",
        first.diagnostics()
    );

    let inventory = server_command(port, &tmp)
        .arg("--orphan-inventory")
        .arg("--orphan-min-age-seconds")
        .arg("3600")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run offline orphan inventory");
    assert!(
        inventory.status.success(),
        "offline inventory should succeed: {}",
        String::from_utf8_lossy(&inventory.stderr)
    );
    let inventory_stdout = String::from_utf8(inventory.stdout).expect("inventory stdout");
    assert!(
        inventory_stdout
            .contains("quackgis_orphan_inventory dry_run=true min_age_seconds=3600 candidates=0")
    );

    let mut reopened =
        ChildGuard::spawn(server_command(port, &tmp), tmp.path().join("reopened.log"));
    reopened.wait_until_listening(port).await;
    assert!(
        reopened
            .signal_and_wait(libc::SIGTERM, "reopened SIGTERM")
            .await
            .success(),
        "reopened server should stop cleanly\n{}",
        reopened.diagnostics()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_mutation_barrier_rejects_partial_and_malformed_startup_config() {
    let port = reserve_port();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cases = [
        ("partial", "delete:before_commit:main.points", false),
        ("malformed", "delete:not_a_stage:main.points", true),
    ];

    for (name, spec, include_markers) in cases {
        let mut command = server_command(port, &tmp);
        command.env(BARRIER_ENV, spec);
        if include_markers {
            command
                .env(BARRIER_READY_ENV, tmp.path().join(format!("{name}.ready")))
                .env(
                    BARRIER_RELEASE_ENV,
                    tmp.path().join(format!("{name}.release")),
                );
        }
        let mut child = ChildGuard::spawn(command, tmp.path().join(format!("{name}.log")));
        let status = child.wait_for_exit(Duration::from_secs(5), name).await;
        assert!(
            !status.success(),
            "{name} native mutation barrier configuration must fail startup"
        );
        let diagnostics = child.diagnostics();
        assert!(
            diagnostics.contains("native mutation") || diagnostics.contains(BARRIER_ENV),
            "{name} startup failure should identify the barrier configuration: {diagnostics}"
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum MutationOperation {
    Delete,
    Update,
    Compact,
}

impl MutationOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Update => "update",
            Self::Compact => "compact",
        }
    }

    fn sql(self, time_bucket: i64, space_bucket: i64) -> String {
        match self {
            Self::Delete => {
                format!("DELETE FROM public.{MUTATION_TABLE} WHERE id = 1 OR id = 2")
            }
            Self::Update => format!(
                "UPDATE public.{MUTATION_TABLE} SET name = 'updated' WHERE id = 1 OR id = 2"
            ),
            Self::Compact => format!(
                "CALL quackgis_compact_table('public.{MUTATION_TABLE}', {time_bucket}, {space_bucket})"
            ),
        }
    }

    fn expected_rows(self) -> Vec<(i32, String)> {
        match self {
            Self::Delete => vec![(3, "c".to_string())],
            Self::Update => vec![
                (1, "updated".to_string()),
                (2, "updated".to_string()),
                (3, "c".to_string()),
            ],
            Self::Compact => baseline_rows(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationStage {
    BeforeCommit,
    AfterCommit,
}

impl MutationStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::BeforeCommit => "before_commit",
            Self::AfterCommit => "after_commit",
        }
    }
}

fn baseline_rows() -> Vec<(i32, String)> {
    vec![
        (1, "a".to_string()),
        (2, "b".to_string()),
        (3, "c".to_string()),
    ]
}

async fn connect(port: u16) -> Client {
    let (client, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        NoTls,
    )
    .await
    .expect("connect to subprocess server");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

async fn seed_mutation_fixture(client: &Client) -> (i64, i64) {
    client
        .batch_execute(&format!(
            "CREATE TABLE public.{MUTATION_TABLE} (
                 id INT,
                 captured_minute INT,
                 geom BINARY,
                 name TEXT
             );
             INSERT INTO public.{MUTATION_TABLE} VALUES
                 (1, 10, X'010100000000000000000000000000000000000000', 'a');
             INSERT INTO public.{MUTATION_TABLE} VALUES
                 (2, 10, X'010100000000000000000000000000000000000000', 'b');
             INSERT INTO public.{MUTATION_TABLE} VALUES
                 (3, 80, X'010100000000000000000010400000000000001040', 'c');"
        ))
        .await
        .expect("seed native mutation process-kill fixture");
    let bucket = client
        .query_one(
            &format!(
                "SELECT _qg_time_bucket, _qg_space_bucket
                 FROM quackgis.main.{MUTATION_TABLE}
                 WHERE id = 1"
            ),
            &[],
        )
        .await
        .expect("read explicit compaction bucket");
    (bucket.get(0), bucket.get(1))
}

async fn query_rows(client: &Client) -> Vec<(i32, String)> {
    client
        .query(
            &format!("SELECT id, name FROM public.{MUTATION_TABLE} ORDER BY id"),
            &[],
        )
        .await
        .expect("query native mutation rows")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

fn parquet_paths(root: &Path) -> BTreeSet<PathBuf> {
    if !root.exists() {
        return BTreeSet::new();
    }
    let mut paths = BTreeSet::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).expect("read data directory") {
            let entry = entry.expect("read data directory entry");
            let file_type = entry.file_type().expect("read data entry type");
            if file_type.is_dir() {
                directories.push(entry.path());
            } else if file_type.is_file() && entry.path().extension() == Some(OsStr::new("parquet"))
            {
                paths.insert(entry.path().canonicalize().expect("canonical Parquet path"));
            }
        }
    }
    paths
}

fn backdate(paths: &BTreeSet<PathBuf>, operation: MutationOperation, stage: MutationStage) {
    let modified = SystemTime::now()
        .checked_sub(Duration::from_secs(5))
        .expect("old modification time");
    for path in paths {
        let file = File::open(path).unwrap_or_else(|err| {
            panic!(
                "{}/{stage:?} open generated path {}: {err}",
                operation.as_str(),
                path.display()
            )
        });
        file.set_times(FileTimes::new().set_modified(modified))
            .unwrap_or_else(|err| {
                panic!(
                    "{}/{stage:?} backdate generated path {}: {err}",
                    operation.as_str(),
                    path.display()
                )
            });
    }
}

fn run_orphan_inventory(
    port: u16,
    tmp: &tempfile::TempDir,
    operation: MutationOperation,
    stage: MutationStage,
) -> BTreeSet<String> {
    let output = server_command(port, tmp)
        .arg("--orphan-inventory")
        .arg("--orphan-min-age-seconds")
        .arg("1")
        .arg("--orphan-show-paths")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|err| {
            panic!(
                "{}/{stage:?} run offline orphan inventory: {err}",
                operation.as_str()
            )
        });
    assert!(
        output.status.success(),
        "{}/{stage:?} orphan inventory failed: {}",
        operation.as_str(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("inventory output is UTF-8");
    let paths = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("orphan_candidate path="))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let expected_header = format!(
        "quackgis_orphan_inventory dry_run=true min_age_seconds=1 candidates={}",
        paths.len()
    );
    assert!(
        stdout.lines().any(|line| line == expected_header),
        "{}/{stage:?} inventory count/path mismatch: {stdout}",
        operation.as_str()
    );
    paths
}

async fn wait_for_barrier(
    child: &mut ChildGuard,
    ready_path: &Path,
    operation: MutationOperation,
    stage: MutationStage,
) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if tokio::fs::try_exists(ready_path)
            .await
            .expect("check ready marker")
        {
            return;
        }
        child.assert_running(&format!("{}/{stage:?} barrier", operation.as_str()));
        assert!(
            Instant::now() < deadline,
            "{}/{stage:?} barrier was not reached; native path may have fallen back\n{}",
            operation.as_str(),
            child.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn run_process_kill_case(operation: MutationOperation, stage: MutationStage) {
    let _serial = MUTATION_PROCESS_TEST_LOCK.lock().await;
    let port = reserve_port();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let ready_path = tmp
        .path()
        .join(format!("{}-{}.ready", operation.as_str(), stage.as_str()));
    let release_path =
        tmp.path()
            .join(format!("{}-{}.release", operation.as_str(), stage.as_str()));
    let mut command = server_command(port, &tmp);
    command
        .env(
            BARRIER_ENV,
            format!(
                "{}:{}:main.{MUTATION_TABLE}",
                operation.as_str(),
                stage.as_str()
            ),
        )
        .env(BARRIER_READY_ENV, &ready_path)
        .env(BARRIER_RELEASE_ENV, &release_path);
    let mut child = ChildGuard::spawn(
        command,
        tmp.path()
            .join(format!("{}-{}.log", operation.as_str(), stage.as_str())),
    );
    child.wait_until_listening(port).await;

    let client = connect(port).await;
    let (time_bucket, space_bucket) = seed_mutation_fixture(&client).await;
    assert_eq!(
        query_rows(&client).await,
        baseline_rows(),
        "{}/{stage:?} fixture baseline",
        operation.as_str()
    );
    let data_path = tmp.path().join("data");
    let baseline_paths = parquet_paths(&data_path);
    let mutation_sql = operation.sql(time_bucket, space_bucket);
    let mutation_task = tokio::spawn(async move { client.batch_execute(&mutation_sql).await });

    wait_for_barrier(&mut child, &ready_path, operation, stage).await;
    let current_paths = parquet_paths(&data_path);
    let generated = current_paths
        .difference(&baseline_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        !generated.is_empty(),
        "{}/{stage:?} barrier produced no new Parquet objects; baseline={baseline_paths:?} current={current_paths:?}",
        operation.as_str()
    );

    let killed = child
        .signal_and_wait(libc::SIGKILL, "native mutation barrier kill")
        .await;
    assert_eq!(
        killed.signal(),
        Some(libc::SIGKILL),
        "{}/{stage:?} child should die from SIGKILL: {killed}",
        operation.as_str()
    );
    let mutation_result = tokio::time::timeout(Duration::from_secs(5), mutation_task)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{}/{stage:?} pgwire mutation did not finish after SIGKILL",
                operation.as_str()
            )
        })
        .expect("mutation task did not panic");
    assert!(
        mutation_result.is_err(),
        "{}/{stage:?} client must not receive a response past the barrier",
        operation.as_str()
    );

    backdate(&generated, operation, stage);
    let inventory = run_orphan_inventory(port, &tmp, operation, stage);
    let generated_strings = generated
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    if stage == MutationStage::BeforeCommit {
        assert_eq!(
            inventory,
            generated_strings,
            "{}/{stage:?} inventory must exactly identify the killed prewrites",
            operation.as_str()
        );
    } else {
        assert!(
            inventory.is_disjoint(&generated_strings),
            "{}/{stage:?} inventory listed committed generated paths: inventory={inventory:?} generated={generated_strings:?}",
            operation.as_str()
        );
        assert!(
            inventory.is_empty(),
            "{}/{stage:?} fresh committed fixture should have no old orphan candidates: {inventory:?}",
            operation.as_str()
        );
    }

    let mut restarted = ChildGuard::spawn(
        server_command(port, &tmp),
        tmp.path().join(format!(
            "{}-{}-restart.log",
            operation.as_str(),
            stage.as_str()
        )),
    );
    restarted.wait_until_listening(port).await;
    let restarted_client = connect(port).await;
    if stage == MutationStage::BeforeCommit {
        assert_eq!(
            query_rows(&restarted_client).await,
            baseline_rows(),
            "{}/{stage:?} restart must expose the old state",
            operation.as_str()
        );
        restarted_client
            .batch_execute(&operation.sql(time_bucket, space_bucket))
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "{}/{stage:?} explicit retry failed: {err}",
                    operation.as_str()
                )
            });
    }
    assert_eq!(
        query_rows(&restarted_client).await,
        operation.expected_rows(),
        "{}/{stage:?} restart/retry visible state",
        operation.as_str()
    );
    assert!(
        restarted
            .signal_and_wait(libc::SIGTERM, "post-assertion SIGTERM")
            .await
            .success(),
        "{}/{stage:?} restarted server should stop cleanly\n{}",
        operation.as_str(),
        restarted.diagnostics()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn process_kill_delete_before_commit_inventories_prewrite_and_retries() {
    run_process_kill_case(MutationOperation::Delete, MutationStage::BeforeCommit).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_kill_update_before_commit_inventories_prewrite_and_retries() {
    run_process_kill_case(MutationOperation::Update, MutationStage::BeforeCommit).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_kill_compact_before_commit_inventories_prewrite_and_retries() {
    run_process_kill_case(MutationOperation::Compact, MutationStage::BeforeCommit).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_kill_delete_after_commit_preserves_committed_state() {
    run_process_kill_case(MutationOperation::Delete, MutationStage::AfterCommit).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_kill_update_after_commit_preserves_committed_state() {
    run_process_kill_case(MutationOperation::Update, MutationStage::AfterCommit).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_kill_compact_after_commit_preserves_committed_state() {
    run_process_kill_case(MutationOperation::Compact, MutationStage::AfterCommit).await;
}
