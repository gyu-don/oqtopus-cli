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
use regex::{Captures, Regex};
use tempfile::TempDir;

#[path = "../support/fake_tool.rs"]
mod fake_tool;
#[path = "../support/http_fixture.rs"]
mod http_fixture;

use fake_tool::FakeTools;
use http_fixture::HttpFixtureServer;

const TEST_VERSION: &str = "9.8.7-characterization";

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
            .envs(extra_env);
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

    format!("exit: {exit_code}\n--- stdout ---\n{stdout}--- stderr ---\n{stderr}")
}

fn normalize(value: &str, sandbox: &Path) -> String {
    static PID: OnceLock<Regex> = OnceLock::new();
    static RFC3339: OnceLock<Regex> = OnceLock::new();
    static DATE_TIME: OnceLock<Regex> = OnceLock::new();
    static TEMP_PATH: OnceLock<Regex> = OnceLock::new();

    let normalized_newlines = value.replace("\r\n", "\n");
    let binary = test_binary().display().to_string();
    let normalized_paths = normalized_newlines
        .replace(&binary, "<TEST_BIN>")
        .replace(&sandbox.display().to_string(), "<TEST_ROOT>")
        .replace(env!("CARGO_MANIFEST_DIR"), "<REPO_ROOT>");
    let normalized_temp_paths = TEMP_PATH
        .get_or_init(|| Regex::new(r#"<TEST_ROOT>/tmp/[^/\s\"]+"#).unwrap())
        .replace_all(&normalized_paths, "<TEST_ROOT>/tmp/<TEMP_DIR>");
    let normalized_pids = PID
        .get_or_init(|| Regex::new(r"(?i)\b(pid\s*[=:]?\s*)\d+\b").unwrap())
        .replace_all(&normalized_temp_paths, |captures: &Captures<'_>| {
            format!("{}<PID>", &captures[1])
        });
    let normalized_rfc3339 = RFC3339
        .get_or_init(|| {
            Regex::new(r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})\b")
                .unwrap()
        })
        .replace_all(&normalized_pids, "<TIMESTAMP>");

    DATE_TIME
        .get_or_init(|| Regex::new(r"\b\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}\b").unwrap())
        .replace_all(&normalized_rfc3339, "<TIMESTAMP>")
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
    format!(
        "{command_output}\n--- filesystem ---\n{}--- selected files ---\n{}",
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
    for (name, args) in [
        ("init", vec!["init", "help"]),
        ("backend", vec!["backend", "help"]),
        ("cloud_local", vec!["cloud-local", "help"]),
        ("completion", vec!["completion", "help"]),
    ] {
        let context = TestContext::new();
        insta::assert_snapshot!(format!("command_help__{name}"), context.run(args, 0));
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
    ] {
        let context = TestContext::new();
        insta::assert_snapshot!(
            format!("representative_errors__{name}"),
            context.run(args, 1)
        );
    }
}

#[test]
fn backend_versions_filters_and_sorts_stable_semver_tags() {
    let context = TestContext::new();
    let server = HttpFixtureServer::new();
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
    let tags_fixture = context.sandbox.path().join("github-tags.json");
    fs::write(&tags_fixture, tags).expect("write implementation-neutral tags fixture");
    server.respond(
        "/repos/oqtopus-team/oqtopus-engine/tags?per_page=100",
        "application/json",
        tags,
    );

    let mut extra_env = vec![("OQTOPUS_TEST_GITHUB_TAGS_FIXTURE", tags_fixture.as_os_str())];
    if let Some(base_url) = server.base_url() {
        extra_env.push(("OQTOPUS_GITHUB_API_BASE_URL", OsStr::new(base_url)));
    }
    let output = context.run_with_env(["backend", "versions", "engine"], 0, extra_env);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("curl"), 1);
        assert_eq!(context.fake_tools.call_count("date"), 0);
        assert_eq!(server.request_count(), 0);
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
    let server = HttpFixtureServer::new();
    let archive = build_backend_template_archive(context.sandbox.path());
    let archive_fixture = context.sandbox.path().join("template-archive.tar.gz");
    fs::write(&archive_fixture, &archive)
        .expect("write implementation-neutral template archive fixture");
    context.fake_tools.fixture("curl").stdout(&archive);
    server.respond(
        "/oqtopus-team/oqtopus-cli/archive/refs/heads/main.tar.gz",
        "application/gzip",
        &archive,
    );
    context
        .fake_tools
        .fixture("date")
        .stdout("2031-12-13T14:15:16Z\n");

    let mut extra_env = vec![
        (
            "OQTOPUS_TEST_TEMPLATE_ARCHIVE_FIXTURE",
            archive_fixture.as_os_str(),
        ),
        ("OQTOPUS_TEST_NOW", OsStr::new("2031-12-13T14:15:16Z")),
    ];
    if let Some(base_url) = server.base_url() {
        extra_env.push(("OQTOPUS_GITHUB_BASE_URL", OsStr::new(base_url)));
    }
    let output = context.run_with_env(["init", "demo", "--template", "backend"], 0, extra_env);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("curl"), 1);
        assert_eq!(context.fake_tools.call_count("date"), 1);
        assert_eq!(server.request_count(), 0);
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
