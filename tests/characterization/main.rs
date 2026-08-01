use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Command as StdCommand, Output, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::assert::OutputAssertExt;
use regex::Regex;
use tempfile::TempDir;

#[path = "../support/fake_tool.rs"]
mod fake_tool;

use fake_tool::FakeTools;

const TEST_VERSION: &str = "9.8.7-characterization";

/// Terminates every snapshot. insta trims trailing whitespace, so without a
/// closing line the final section's trailing newline would go unrecorded.
const END_SENTINEL: &str = "--- end ---\n";

struct TestContext {
    binary: PathBuf,
    invoke_with_bash: bool,
    sandbox: TempDir,
    fake_tools: FakeTools,
}

impl TestContext {
    fn new() -> Self {
        let binary = test_binary();
        assert!(
            binary.is_file(),
            "OQTOPUS test binary does not exist or is not a file: {}",
            binary.display()
        );

        let sandbox = tempfile::tempdir().expect("create isolated test directory");
        let fake_tools = FakeTools::new(sandbox.path());

        Self {
            binary,
            invoke_with_bash: env::var_os("OQTOPUS_TEST_BIN").is_none(),
            sandbox,
            fake_tools,
        }
    }

    fn run<I, S>(&self, args: I, expected_code: i32) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with_env(args, expected_code, std::iter::empty::<(&str, &str)>())
    }

    fn run_with_env<I, S, E, K, V>(&self, args: I, expected_code: i32, extra_env: E) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let root = self.sandbox.path();
        let home = root.join("home");
        let data = root.join("xdg-data");
        let config = root.join("xdg-config");
        let cache = root.join("xdg-cache");
        let state = root.join("xdg-state");
        let runtime = root.join("xdg-runtime");
        let tmp = root.join("tmp");
        let work = root.join("work");

        for directory in [&home, &data, &config, &cache, &state, &runtime, &tmp, &work] {
            std::fs::create_dir_all(directory).expect("create isolated environment directory");
        }

        let path = self
            .fake_tools
            .path_with(env::var_os("PATH").expect("PATH is required to execute the shell CLI"));
        // The checked-in Bash script deliberately does not need an executable bit.
        // An explicitly configured candidate is treated as a native executable.
        let mut command = if self.invoke_with_bash {
            let mut command = Command::new("bash");
            command.arg(&self.binary);
            command
        } else {
            Command::new(&self.binary)
        };

        command
            .args(args)
            .current_dir(&work)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &home)
            .env("TMPDIR", &tmp)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_CACHE_HOME", &cache)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("OQTOPUS_CLI_VERSION", TEST_VERSION)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("TZ", "UTC")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("http_proxy", "http://127.0.0.1:1")
            .env("https_proxy", "http://127.0.0.1:1")
            .env("all_proxy", "http://127.0.0.1:1")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .envs(self.fake_tools.env())
            .envs(extra_env)
            // The CLI is expected never to prompt. Closing stdin turns a future
            // interactive read into an immediate EOF instead of a 20s timeout.
            .stdin(Stdio::null());
        let output = output_with_timeout(command, Duration::from_secs(20))
            .assert()
            .code(expected_code)
            .get_output()
            .clone();

        render_output(&output, root)
    }

    fn work_dir(&self) -> PathBuf {
        self.sandbox.path().join("work")
    }
}

fn output_with_timeout(mut command: Command, timeout: Duration) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn CLI under test");
    let mut stdout = child.stdout.take().expect("capture CLI stdout");
    let mut stderr = child.stderr.take().expect("capture CLI stderr");
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).expect("read CLI stdout");
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).expect("read CLI stderr");
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll CLI under test") {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill().expect("kill timed-out CLI under test");
            let _ = child.wait();
            panic!("CLI under test exceeded {timeout:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };

    Output {
        status,
        stdout: stdout_reader.join().expect("join CLI stdout reader"),
        stderr: stderr_reader.join().expect("join CLI stderr reader"),
    }
}

fn test_binary() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let configured = env::var_os("OQTOPUS_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("bin/oqtopus"));

    if configured.is_absolute() {
        configured
    } else {
        env::current_dir()
            .expect("resolve current directory")
            .join(configured)
    }
}

fn render_output(output: &Output, sandbox: &Path) -> String {
    let stdout = normalize(&String::from_utf8_lossy(&output.stdout), sandbox);
    let stderr = normalize(&String::from_utf8_lossy(&output.stderr), sandbox);
    let exit_code = output
        .status
        .code()
        .map_or_else(|| "<SIGNAL>".to_owned(), |code| code.to_string());

    format!("exit: {exit_code}\n--- stdout ---\n{stdout}--- stderr ---\n{stderr}{END_SENTINEL}")
}

fn normalize(value: &str, sandbox: &Path) -> String {
    static TEMP_PATH: OnceLock<Regex> = OnceLock::new();

    let normalized_newlines = value.replace("\r\n", "\n");
    let binary = test_binary().display().to_string();
    let normalized_paths = normalized_newlines
        .replace(&binary, "<TEST_BIN>")
        .replace(&sandbox.display().to_string(), "<TEST_ROOT>")
        .replace(env!("CARGO_MANIFEST_DIR"), "<REPO_ROOT>");

    TEMP_PATH
        .get_or_init(|| Regex::new(r#"<TEST_ROOT>/tmp/[^/\s\"]+"#).unwrap())
        .replace_all(&normalized_paths, "<TEST_ROOT>/tmp/<TEMP_DIR>")
        .into_owned()
}

fn render_tree(root: &Path) -> String {
    fn visit(root: &Path, directory: &Path, entries: &mut Vec<String>) {
        let mut children: Vec<_> = fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read tree directory {}: {error}", directory.display()))
            .map(|entry| entry.expect("read tree entry"))
            .collect();
        children.sort_by_key(|entry| entry.file_name());

        for entry in children {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("tree entry is below root");
            let kind = entry.file_type().expect("read tree entry type");
            if kind.is_dir() {
                entries.push(format!("{}/", relative.display()));
                visit(root, &path, entries);
            } else if kind.is_file() {
                entries.push(relative.display().to_string());
            } else if kind.is_symlink() {
                entries.push(format!("{} -> <SYMLINK>", relative.display()));
            }
        }
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    if entries.is_empty() {
        "<EMPTY>\n".to_owned()
    } else {
        format!("{}\n", entries.join("\n"))
    }
}

fn render_selected_files(root: &Path, paths: &[&str], sandbox: &Path) -> String {
    if paths.is_empty() {
        return "<NONE>\n".to_owned();
    }

    paths
        .iter()
        .map(|relative| {
            let value = fs::read_to_string(root.join(relative))
                .unwrap_or_else(|error| panic!("read selected file {relative}: {error}"));
            format!("--- {relative} ---\n{}", normalize(&value, sandbox))
        })
        .collect::<Vec<_>>()
        .join("")
}

fn render_observation(
    context: &TestContext,
    command_output: &str,
    selected_files: &[&str],
) -> String {
    let sandbox = context.sandbox.path();
    let work = context.work_dir();
    // Splice the extra sections in ahead of the sentinel so it stays last, and
    // so no separator is inserted that would mask stderr's trailing newline.
    let body = command_output
        .strip_suffix(END_SENTINEL)
        .expect("command output is terminated by the end sentinel");
    format!(
        "{body}--- filesystem ---\n{}--- selected files ---\n{}{END_SENTINEL}",
        render_tree(&work),
        render_selected_files(&work, selected_files, sandbox)
    )
}

fn build_backend_template_archive(root: &Path) -> Vec<u8> {
    let source = root.join("archive-source");
    let template = source.join("oqtopus-cli-main/templates/backend");
    fs::create_dir_all(template.join("config/nested"))
        .expect("create backend template fixture directories");
    fs::write(
        template.join("config/.env"),
        "ENV_NAME={{ env_name }}\nAPI_URL=http://{{ env_name }}.example.test\n",
    )
    .expect("write backend environment template fixture");
    fs::write(
        template.join("config/nested/backend.toml"),
        "mode = \"fixture\"\n",
    )
    .expect("write nested backend fixture");
    fs::write(template.join("compose.yaml"), "name: characterization\n")
        .expect("write backend compose fixture");

    let archive = root.join("backend-template.tar.gz");
    let status = StdCommand::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&source)
        .arg("oqtopus-cli-main")
        .status()
        .expect("run tar to build local backend template fixture");
    assert!(status.success(), "tar failed to build template fixture");
    fs::read(archive).expect("read backend template fixture archive")
}

#[test]
fn top_level_help() {
    for (name, args) in [
        ("no_arguments", Vec::<&str>::new()),
        ("help", vec!["help"]),
        ("help_flag", vec!["--help"]),
    ] {
        let context = TestContext::new();
        insta::assert_snapshot!(format!("top_level_help__{name}"), context.run(args, 0));
    }
}

#[test]
fn command_help() {
    for (name, command) in [
        ("init", vec!["init"]),
        ("backend", vec!["backend"]),
        ("cloud_local", vec!["cloud-local"]),
        ("completion", vec!["completion"]),
        ("backend_install", vec!["backend", "install"]),
        ("backend_build", vec!["backend", "build"]),
        ("backend_versions", vec!["backend", "versions"]),
        ("backend_uninstall", vec!["backend", "uninstall"]),
        ("backend_update", vec!["backend", "update"]),
        ("backend_start", vec!["backend", "start"]),
        ("backend_stop", vec!["backend", "stop"]),
        ("backend_restart", vec!["backend", "restart"]),
        ("backend_device_status", vec!["backend", "device-status"]),
        ("cloud_local_install", vec!["cloud-local", "install"]),
        ("cloud_local_versions", vec!["cloud-local", "versions"]),
        ("cloud_local_uninstall", vec!["cloud-local", "uninstall"]),
        ("cloud_local_update", vec!["cloud-local", "update"]),
        ("cloud_local_start", vec!["cloud-local", "start"]),
        ("cloud_local_stop", vec!["cloud-local", "stop"]),
        ("cloud_local_restart", vec!["cloud-local", "restart"]),
    ] {
        let display = command.join(" ");
        let help_context = TestContext::new();
        let mut help_args = command.clone();
        help_args.push("help");
        let help_output = help_context.run(help_args, 0);
        insta::assert_snapshot!(format!("command_help__{name}"), &help_output);

        let flag_context = TestContext::new();
        let mut flag_args = command;
        flag_args.push("--help");
        let flag_output = flag_context.run(flag_args, 0);
        assert_eq!(
            help_output, flag_output,
            "`{display}` help and --help must agree"
        );
    }
}

#[test]
fn dispatcher_without_arguments_prints_help() {
    for command in ["backend", "cloud-local"] {
        let help_context = TestContext::new();
        let help_output = help_context.run([command, "help"], 0);

        let bare_context = TestContext::new();
        let bare_output = bare_context.run([command], 0);
        assert_eq!(
            help_output, bare_output,
            "`{command}` with no arguments must match `{command} help`"
        );
    }
}

#[test]
fn version() {
    for (name, argument) in [("command", "version"), ("flag", "--version")] {
        let context = TestContext::new();
        let output = context.run([argument], 0);
        assert_eq!(context.fake_tools.call_count("curl"), 0);
        insta::assert_snapshot!(format!("version__{name}"), output);
    }
}

#[test]
fn completion_scripts() {
    for shell in ["bash", "zsh", "fish"] {
        let context = TestContext::new();
        insta::assert_snapshot!(
            format!("completion_scripts__{shell}"),
            context.run(["completion", shell], 0)
        );
    }
}

#[test]
fn representative_errors() {
    for (name, args) in [
        ("unknown_top_level_command", vec!["unknown"]),
        ("unknown_backend_command", vec!["backend", "unknown"]),
        (
            "unknown_cloud_local_command",
            vec!["cloud-local", "unknown"],
        ),
        (
            "unsupported_completion_shell",
            vec!["completion", "powershell"],
        ),
        (
            "backend_status_outside_environment",
            vec!["backend", "status"],
        ),
        ("init_no_arguments", vec!["init"]),
        ("init_missing_template_flag", vec!["init", "demo"]),
        (
            "init_template_flag_without_value",
            vec!["init", "demo", "--template"],
        ),
        (
            "init_extra_argument",
            vec!["init", "demo", "--template", "backend", "extra"],
        ),
        ("backend_versions_no_arguments", vec!["backend", "versions"]),
        (
            "backend_versions_extra_argument",
            vec!["backend", "versions", "engine", "extra"],
        ),
        ("completion_no_arguments", vec!["completion"]),
        (
            "completion_extra_argument",
            vec!["completion", "bash", "zsh"],
        ),
        (
            "backend_status_extra_argument",
            vec!["backend", "status", "extra"],
        ),
        (
            "backend_info_extra_argument",
            vec!["backend", "info", "extra"],
        ),
    ] {
        let context = TestContext::new();
        insta::assert_snapshot!(
            format!("representative_errors__{name}"),
            context.run(args, 1)
        );
    }
}

#[test]
fn init_rejects_invalid_requests_without_side_effects() {
    for (name, args) in [
        (
            "init_rejects__invalid_env_name",
            vec!["init", "Bad_Name", "--template", "backend"],
        ),
        (
            "init_rejects__unknown_template",
            vec!["init", "demo", "--template", "nonexistent"],
        ),
    ] {
        let context = TestContext::new();
        let output = context.run(args, 1);
        insta::assert_snapshot!(name, render_observation(&context, &output, &[]));
    }
}

#[test]
fn backend_versions_filters_and_sorts_stable_semver_tags() {
    let context = TestContext::new();
    let tags = r#"[
  {"name":"v1.2.3"},
  {"name":"v2.0.0-rc.1"},
  {"name":"v10.0.0"},
  {"name":"nightly"},
  {"name":"v1.10.0"},
  {"name":"v2.0.0"},
  {"name":"v0.9.9"},
  {"name":"v2.0.0"},
  {"name":"release-v4.0.0"}
]"#;
    context.fake_tools.fixture("curl").stdout(tags);

    let output = context.run(["backend", "versions", "engine"], 0);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("curl"), 1);
        assert_eq!(context.fake_tools.call_count("date"), 0);
        insta::assert_snapshot!(
            "bash_external_calls__backend_versions",
            normalize(&context.fake_tools.log(), context.sandbox.path())
        );
    }

    insta::assert_snapshot!(
        "backend_versions_filters_and_sorts_stable_semver_tags",
        render_observation(&context, &output, &[])
    );
}

#[test]
fn init_backend_creates_rendered_environment() {
    let context = TestContext::new();
    let archive = build_backend_template_archive(context.sandbox.path());
    context.fake_tools.fixture("curl").stdout(&archive);
    context
        .fake_tools
        .fixture("date")
        .stdout("2031-12-13T14:15:16Z\n");

    let output = context.run(["init", "demo", "--template", "backend"], 0);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("curl"), 1);
        assert_eq!(context.fake_tools.call_count("date"), 1);
        insta::assert_snapshot!(
            "bash_external_calls__init_backend",
            normalize(&context.fake_tools.log(), context.sandbox.path())
        );
    }

    insta::assert_snapshot!(
        "init_backend_creates_rendered_environment",
        render_observation(
            &context,
            &output,
            &[
                "demo/.metadata",
                "demo/config/.env",
                "demo/config/nested/backend.toml",
                "demo/compose.yaml",
            ],
        )
    );
}
