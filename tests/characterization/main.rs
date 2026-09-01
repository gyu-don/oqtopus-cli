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
    // Owns the temporary directory; every path is taken from `root` instead.
    _sandbox: TempDir,
    root: PathBuf,
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
        // The CLI compares `.metadata`'s env_root against `pwd -P`, and the fake
        // tools compare their cwd against the sandbox root; both report the
        // physical path. macOS places temporary directories under /var, which is
        // a symlink to /private/var, so an unresolved sandbox path would never
        // compare equal there. Resolve it once, up front.
        let root = fs::canonicalize(sandbox.path()).expect("resolve the sandbox path");
        let fake_tools = FakeTools::new(&root);

        Self {
            binary,
            invoke_with_bash: env::var_os("OQTOPUS_TEST_BIN").is_none(),
            _sandbox: sandbox,
            root,
            fake_tools,
        }
    }

    fn root(&self) -> &Path {
        &self.root
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
        let root = self.root.as_path();
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
            // Absolute path on purpose: a test may replace PATH outright to make
            // a command genuinely absent, and that must not hide the interpreter.
            let mut command = Command::new(bash_program());
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
        self.root.join("work")
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

/// Resolves `bash` against the ambient PATH once, before any test narrows the
/// PATH handed to the CLI.
fn bash_program() -> &'static Path {
    static BASH: OnceLock<PathBuf> = OnceLock::new();
    BASH.get_or_init(|| {
        let path = env::var_os("PATH").expect("PATH is required to locate bash");
        env::split_paths(&path)
            .map(|directory| directory.join("bash"))
            .find(|candidate| candidate.is_file())
            .expect("bash was not found on PATH")
    })
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
    static BUILD_ARG_ID: OnceLock<Regex> = OnceLock::new();

    let normalized_newlines = value.replace("\r\n", "\n");
    let binary = test_binary().display().to_string();
    let normalized_paths = normalized_newlines
        .replace(&binary, "<TEST_BIN>")
        .replace(&sandbox.display().to_string(), "<TEST_ROOT>")
        .replace(env!("CARGO_MANIFEST_DIR"), "<REPO_ROOT>");
    let normalized_temp_paths = TEMP_PATH
        .get_or_init(|| Regex::new(r#"<TEST_ROOT>/tmp/[^/\s\"]+"#).unwrap())
        .replace_all(&normalized_paths, "<TEST_ROOT>/tmp/<TEMP_DIR>");

    // build_sse_runtime passes id -u and id -g through to docker. Those differ
    // per machine and per CI runner image, so the values cannot be recorded;
    // that they are passed at all is the part that matters.
    BUILD_ARG_ID
        .get_or_init(|| Regex::new(r"\b(UID|GID)=\d+\b").unwrap())
        .replace_all(&normalized_temp_paths, "$1=<ID>")
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
    let sandbox = context.root();
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

/// The on-disk shape `oqtopus init` leaves behind, built directly.
///
/// Commands that run *inside* an environment need one to exist. Producing it
/// by running `init` first would couple every such test to the template
/// download, so the layout is written here instead.
struct EnvFixture {
    install_root: PathBuf,
}

/// Which generation of `.metadata` key names a fixture is written with.
enum KeyStyle {
    Current,
    Legacy,
}

impl EnvFixture {
    /// Writes `.metadata` for a `backend` environment into the work directory.
    /// `extra` lines are appended in the given order, so a caller can control
    /// key order and deliberately introduce duplicates.
    fn backend(context: &TestContext, extra: &[(&str, &str)]) -> Self {
        Self::create(context, "backend", "backend", extra)
    }

    fn create(
        context: &TestContext,
        template: &str,
        data_dir: &str,
        extra: &[(&str, &str)],
    ) -> Self {
        Self::create_with_key_style(context, template, data_dir, extra, KeyStyle::Current)
    }

    /// Like `create`, but written with the pre-rename `env_name`/`env_root`
    /// keys that environments created by older CLI releases still carry.
    fn create_legacy(
        context: &TestContext,
        template: &str,
        data_dir: &str,
        extra: &[(&str, &str)],
    ) -> Self {
        Self::create_with_key_style(context, template, data_dir, extra, KeyStyle::Legacy)
    }

    fn create_with_key_style(
        context: &TestContext,
        template: &str,
        data_dir: &str,
        extra: &[(&str, &str)],
        key_style: KeyStyle,
    ) -> Self {
        let env_root = context.work_dir();
        // Mirrors data_root()/cloud_local_data_root() under the harness's XDG_DATA_HOME.
        let install_root = context
            .root()
            .join("xdg-data/oqtopus")
            .join(data_dir)
            .join("releases");
        fs::create_dir_all(&env_root).expect("create the environment root");
        fs::create_dir_all(&install_root).expect("create the install root");

        let (name_key, root_key) = match key_style {
            KeyStyle::Current => ("environment_name", "environment_root"),
            KeyStyle::Legacy => ("env_name", "env_root"),
        };
        let mut metadata = format!(
            "template={template}\ninstall_root={}\n{name_key}=demo\n{root_key}={}\ncreated_at=2031-12-13T14:15:16Z\n",
            install_root.display(),
            env_root.display()
        );
        for (key, value) in extra {
            metadata.push_str(&format!("{key}={value}\n"));
        }
        fs::write(env_root.join(".metadata"), metadata).expect("write .metadata");

        Self { install_root }
    }

    /// Creates `<install_root>/<name>`, as an extracted release would leave it.
    fn install_release(&self, name: &str) -> PathBuf {
        let path = self.install_root.join(name);
        fs::create_dir_all(&path)
            .unwrap_or_else(|error| panic!("create release {}: {error}", path.display()));
        path
    }
}

/// Packs `<root>/archive-source/<top_level>` (which `populate` must fill in)
/// into a gzipped tarball, as GitHub's codeload archives are shaped: a single
/// top-level directory wrapping the repository contents.
fn build_targz(root: &Path, top_level: &str, populate: impl FnOnce(&Path)) -> Vec<u8> {
    let source = root.join("archive-source");
    let contents = source.join(top_level);
    fs::create_dir_all(&contents).expect("create archive fixture directory");
    populate(&contents);

    let archive = root.join("fixture-archive.tar.gz");
    let status = StdCommand::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&source)
        .arg(top_level)
        .status()
        .expect("run tar to build archive fixture");
    assert!(status.success(), "tar failed to build archive fixture");
    fs::remove_dir_all(&source).expect("remove archive fixture source");
    fs::read(archive).expect("read archive fixture")
}

fn build_backend_template_archive(root: &Path) -> Vec<u8> {
    build_targz(root, "oqtopus-cli-main", |contents| {
        let template = contents.join("templates/backend");
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
    })
}

/// A fabricated 40-hex object id that is stable per ref name, so snapshots are
/// deterministic and a recorded id can be traced back to the ref it stands for.
fn fake_commit_id(refname: &str) -> String {
    let mut id = String::with_capacity(40);
    for byte in refname.bytes().cycle() {
        id.push_str(&format!("{byte:02x}"));
        if id.len() >= 40 {
            break;
        }
    }
    id.truncate(40);
    id
}

/// Renders the response body of `GET <repo>.git/info/refs?service=git-upload-pack`,
/// which the CLI's tag listing and branch resolution parse. The framing follows
/// a real git smart-HTTP server: pkt-line length prefixes, a `# service`
/// preamble, a NUL-separated capability list on the first ref line, and flush
/// packets — all of which the CLI's parser must tolerate.
fn upload_pack_advertisement(refs: &[(String, String)]) -> Vec<u8> {
    fn pkt_line(payload: &[u8]) -> Vec<u8> {
        let mut line = format!("{:04x}", payload.len() + 4).into_bytes();
        line.extend_from_slice(payload);
        line
    }

    let mut body = pkt_line(b"# service=git-upload-pack\n");
    body.extend_from_slice(b"0000");
    for (index, (commit_id, refname)) in refs.iter().enumerate() {
        let mut payload = format!("{commit_id} {refname}").into_bytes();
        if index == 0 {
            payload.extend_from_slice(b"\0multi_ack symref=HEAD:refs/heads/main agent=git/fixture");
        }
        payload.push(b'\n');
        body.extend_from_slice(&pkt_line(&payload));
    }
    body.extend_from_slice(b"0000");
    body
}

/// An advertisement carrying HEAD, `refs/heads/main`, and the given refs
/// (`heads/...` or `tags/...`), with fabricated per-ref commit ids.
fn advertised_refs(refs: &[&str]) -> Vec<u8> {
    let mut lines = vec![
        (fake_commit_id("refs/heads/main"), "HEAD".to_owned()),
        (
            fake_commit_id("refs/heads/main"),
            "refs/heads/main".to_owned(),
        ),
    ];
    for name in refs {
        let refname = format!("refs/{name}");
        lines.push((fake_commit_id(&refname), refname));
    }
    upload_pack_advertisement(&lines)
}

/// An advertisement whose only interesting refs are the given tags.
fn advertised_tags(tags: &[&str]) -> Vec<u8> {
    let refs: Vec<String> = tags.iter().map(|tag| format!("tags/{tag}")).collect();
    advertised_refs(&refs.iter().map(String::as_str).collect::<Vec<_>>())
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
    // Branch refs and the peeled `^{}` companion of an annotated tag must be
    // ignored; non-semver and pre-release tags must be filtered out.
    let refs = advertised_refs(&[
        "heads/feature/x",
        "tags/v1.2.3",
        "tags/v2.0.0-rc.1",
        "tags/v10.0.0",
        "tags/nightly",
        "tags/v1.10.0",
        "tags/v2.0.0",
        "tags/v2.0.0^{}",
        "tags/v0.9.9",
        "tags/release-v4.0.0",
    ]);
    context.fake_tools.fixture("curl").stdout(refs);

    let output = context.run(["backend", "versions", "engine"], 0);

    if context.invoke_with_bash {
        assert_eq!(context.fake_tools.call_count("curl"), 1);
        assert_eq!(context.fake_tools.call_count("date"), 0);
        insta::assert_snapshot!(
            "bash_external_calls__backend_versions",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    insta::assert_snapshot!(
        "backend_versions_filters_and_sorts_stable_semver_tags",
        render_observation(&context, &output, &[])
    );
}

#[test]
fn backend_versions_annotates_environment_context() {
    let tags = advertised_tags(&[
        "v1.2.3",
        "v2.0.0",
        "v1.10.0",
        "v0.9.9",
        "v2.0.0-rc.1",
        "nightly",
    ]);

    // The current binding is the only difference between the two cases: it moves
    // the `* ` marker, and a branch binding sorts ahead of every semver tag.
    for (name, engine_version) in [
        ("backend_versions__current_release", "v1.10.0"),
        ("backend_versions__current_branch", "branch:feature/x"),
    ] {
        let context = TestContext::new();
        let env = EnvFixture::backend(&context, &[("engine_version", engine_version)]);
        for release in [
            "engine-v1.2.3",
            "engine-v1.10.0",
            "engine-v3.3.3",
            "engine-vfoo",
            "tranqu-v1.0.0",
        ] {
            env.install_release(release);
        }
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["backend", "versions", "engine"], 0);
        assert_eq!(context.fake_tools.call_count("curl"), 1);

        insta::assert_snapshot!(name, output);
    }
}

#[test]
fn backend_versions_sorts_patch_releases_numerically() {
    // 100 tags with a shared major.minor so the only thing distinguishing
    // entries is `.patch`. This is deliberately "adversarial" for a naive
    // lexicographic sort: v1.0.99 < v1.0.9 as strings, but must sort *after*
    // it numerically. A full descending dump of all 100 versions is the only
    // way to confirm the sort is numeric end-to-end (e.g. that v1.0.10 lands
    // between v1.0.11 and v1.0.9, not next to v1.0.1).
    let tags: Vec<String> = (0..100).map(|patch| format!("v1.0.{patch}")).collect();
    let refs = advertised_tags(&tags.iter().map(String::as_str).collect::<Vec<_>>());

    let context = TestContext::new();
    context.fake_tools.fixture("curl").stdout(refs);

    let output = context.run(["backend", "versions", "gateway"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 1);

    insta::assert_snapshot!("backend_versions_sorts_patch_releases_numerically", output);
}

#[test]
fn version_resolves_latest_when_cli_version_is_empty() {
    // Same adversarial patch-release fixture as list_component_versions, but
    // exercised through `resolve_latest_version`, which must pick the numeric
    // maximum as "latest".
    let tags: Vec<String> = (0..100).map(|patch| format!("v1.0.{patch}")).collect();
    let refs = advertised_tags(&tags.iter().map(String::as_str).collect::<Vec<_>>());

    let context = TestContext::new();
    context.fake_tools.fixture("curl").stdout(refs);

    // OQTOPUS_CLI_VERSION is always set by the harness; override it with the
    // empty string so `cli_version()` falls through to `resolve_latest_version`.
    let output = context.run_with_env(["version"], 0, [("OQTOPUS_CLI_VERSION", "")]);
    assert_eq!(context.fake_tools.call_count("curl"), 1);

    insta::assert_snapshot!("version_resolves_latest_when_cli_version_is_empty", output);
}

#[test]
fn backend_versions_rejects_unusable_input() {
    let context = TestContext::new();
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["nightly"]));
    let output = context.run(["backend", "versions", "tranqu"], 1);
    assert_eq!(context.fake_tools.call_count("curl"), 1);
    insta::assert_snapshot!("backend_versions__no_stable_versions", output);

    // `is_in_list` rejects the component before `need_command curl` ever runs,
    // so no curl fixture is configured here and none must be called.
    let context = TestContext::new();
    let output = context.run(["backend", "versions", "bogus"], 1);
    assert_eq!(context.fake_tools.call_count("curl"), 0);
    insta::assert_snapshot!("backend_versions__unknown_component", output);
}

#[test]
fn init_backend_creates_rendered_environment() {
    let context = TestContext::new();
    let archive = build_backend_template_archive(context.root());
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
            normalize(&context.fake_tools.log(), context.root())
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

#[test]
fn backend_info_reports_metadata_verbatim() {
    let context = TestContext::new();
    EnvFixture::backend(&context, &[]);

    let output = context.run(["backend", "info"], 0);

    insta::assert_snapshot!("backend_info__metadata", output);
}

#[test]
fn backend_environment_validation_errors() {
    // No `.metadata` at all.
    {
        let context = TestContext::new();
        let output = context.run(["backend", "info"], 1);
        insta::assert_snapshot!("backend_env__no_metadata", output);
    }

    // `.metadata` exists but belongs to a different template.
    {
        let context = TestContext::new();
        let work = context.work_dir();
        fs::create_dir_all(&work).expect("create work dir");
        fs::write(
            work.join(".metadata"),
            format!(
                "template=cloud-local\nenvironment_root={}\ninstall_root={}\n",
                work.display(),
                context
                    .root()
                    .join("xdg-data/oqtopus/backend/releases")
                    .display(),
            ),
        )
        .expect("write .metadata");
        let output = context.run(["backend", "info"], 1);
        insta::assert_snapshot!("backend_env__wrong_template", output);
    }

    // `template=backend` and a correct `environment_root`, but no `install_root`.
    {
        let context = TestContext::new();
        let work = context.work_dir();
        fs::create_dir_all(&work).expect("create work dir");
        fs::write(
            work.join(".metadata"),
            format!("template=backend\nenvironment_root={}\n", work.display()),
        )
        .expect("write .metadata");
        let output = context.run(["backend", "info"], 1);
        insta::assert_snapshot!("backend_env__missing_install_root", output);
    }

    // `environment_root` in `.metadata` does not match the current directory.
    {
        let context = TestContext::new();
        let work = context.work_dir();
        fs::create_dir_all(&work).expect("create work dir");
        let mismatched_env_root = context.root().join("elsewhere");
        fs::write(
            work.join(".metadata"),
            format!(
                "template=backend\ninstall_root={}\nenvironment_root={}\n",
                context
                    .root()
                    .join("xdg-data/oqtopus/backend/releases")
                    .display(),
                mismatched_env_root.display(),
            ),
        )
        .expect("write .metadata");
        let output = context.run(["backend", "info"], 1);
        insta::assert_snapshot!("backend_env__env_root_mismatch", output);
    }
}

#[test]
fn legacy_metadata_keys_are_migrated_in_place() {
    // Environments created by older CLI releases carry `env_name`/`env_root`.
    // Any command that validates the environment (here: `backend info`) must
    // both accept them and rewrite the file to the new key names.
    let context = TestContext::new();
    EnvFixture::create_legacy(&context, "backend", "backend", &[]);

    let output = context.run(["backend", "info"], 0);

    insta::assert_snapshot!(
        "legacy_metadata_keys__migrated_by_backend_info",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn legacy_metadata_keys_are_read_but_not_migrated_without_validation() {
    // `backend versions` only *tries* to load the environment, and the
    // read-only path must not rewrite `.metadata` as a side effect.
    let context = TestContext::new();
    EnvFixture::create_legacy(
        &context,
        "backend",
        "backend",
        &[("engine_version", "v1.2.3")],
    );
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["v1.2.3", "v2.0.0"]));

    let output = context.run(["backend", "versions", "engine"], 0);

    insta::assert_snapshot!(
        "legacy_metadata_keys__read_only_by_backend_versions",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn backend_versions_ignores_unusable_metadata() {
    let context = TestContext::new();
    let work = context.work_dir();
    fs::create_dir_all(&work).expect("create work dir");
    // Wrong template: `try_load_backend_env` must reject this silently rather
    // than warn, since "versions" works fine outside any environment too.
    fs::write(
        work.join(".metadata"),
        format!(
            "template=cloud-local\nenvironment_root={}\n",
            work.display()
        ),
    )
    .expect("write .metadata");
    context
        .fake_tools
        .fixture("curl")
        .stdout(advertised_tags(&["v1.2.3", "v2.0.0"]));

    let output = context.run(["backend", "versions", "engine"], 0);

    insta::assert_snapshot!("backend_versions__unusable_metadata", output);
}

#[test]
fn metadata_get_uses_the_first_match_and_keeps_inner_separators() {
    let tags = advertised_tags(&["v1.2.3", "v2.0.0"]);

    // Two `engine_version` lines in `.metadata`: the first one must win.
    {
        let context = TestContext::new();
        EnvFixture::backend(
            &context,
            &[("engine_version", "v1.2.3"), ("engine_version", "v2.0.0")],
        );
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["backend", "versions", "engine"], 0);

        insta::assert_snapshot!("metadata_get__duplicate_key", output);
    }

    // A value that itself contains `=`: everything after the first `=` on the
    // line is the value.
    {
        let context = TestContext::new();
        EnvFixture::backend(&context, &[("engine_version", "branch:a=b")]);
        context.fake_tools.fixture("curl").stdout(&tags);

        let output = context.run(["backend", "versions", "engine"], 0);

        insta::assert_snapshot!("metadata_get__value_contains_separator", output);
    }
}

#[test]
fn backend_install_branch_writes_metadata_binding() {
    let context = TestContext::new();
    EnvFixture::backend(&context, &[]);
    // A branch install makes two curl calls: first the ref advertisement that
    // resolves the branch to a commit id, then the codeload tarball for that
    // commit (whose single top-level directory gets stripped on extraction).
    context
        .fake_tools
        .fixture_call("curl", 1)
        .stdout(advertised_refs(&[]));
    let checkout = build_targz(context.root(), "tranqu-server-checkout", |contents| {
        fs::write(
            contents.join("pyproject.toml"),
            "[project]\nname = \"tranqu\"\n",
        )
        .expect("write tranqu pyproject fixture");
    });
    context.fake_tools.fixture_call("curl", 2).stdout(checkout);
    context.fake_tools.fixture("uv");

    let output = context.run(["backend", "install", "tranqu", "branch:main"], 0);
    assert_eq!(context.fake_tools.call_count("curl"), 2);

    if context.invoke_with_bash {
        insta::assert_snapshot!(
            "bash_external_calls__backend_install_branch",
            normalize(&context.fake_tools.log(), context.root())
        );
    }

    insta::assert_snapshot!(
        "backend_install__branch_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}

#[test]
fn backend_uninstall_branch_clears_metadata_binding() {
    let context = TestContext::new();
    EnvFixture::backend(&context, &[("tranqu_version", "branch:main")]);
    fs::create_dir_all(context.work_dir().join("tranqu")).expect("create tranqu branch checkout");

    let output = context.run(["backend", "uninstall", "tranqu", "branch:main"], 0);

    insta::assert_snapshot!(
        "backend_uninstall__branch_binding",
        render_observation(&context, &output, &[".metadata"])
    );
}

/// Lays out a backend environment with an installed `engine` release ready
/// for `backend build sse-runtime`, and writes `config/.env` with `contents`.
fn setup_sse_runtime_build(context: &TestContext, env_dot_env_contents: &str) {
    let env = EnvFixture::backend(context, &[("engine_version", "v1.2.3")]);
    let release = env.install_release("engine-v1.2.3");
    for project in ["core", "combiner", "estimator", "mitigator"] {
        fs::create_dir_all(release.join(project).join(".venv"))
            .unwrap_or_else(|error| panic!("create {project}/.venv: {error}"));
    }
    fs::create_dir_all(release.join("sse_runtime")).expect("create sse_runtime dir");
    fs::write(
        release.join("sse_runtime").join("Dockerfile"),
        "FROM scratch\n",
    )
    .expect("write sse_runtime Dockerfile");

    let config_dir = context.work_dir().join("config");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join(".env"), env_dot_env_contents).expect("write config/.env");

    context.fake_tools.fixture("docker");
}

#[test]
fn sse_runtime_build_reads_the_env_config() {
    let success_cases = [
        ("double_quoted", "SSE_CONTAINER_IMAGE=\"img:1\"\n"),
        ("single_quoted", "SSE_CONTAINER_IMAGE='img:2'\n"),
        ("value_contains_separator", "SSE_CONTAINER_IMAGE=a=b=c\n"),
        (
            "after_comments",
            "# SSE_CONTAINER_IMAGE=ignored\n   # indented comment\n\nSSE_CONTAINER_IMAGE=img:5\n",
        ),
        ("empty_value", "SSE_CONTAINER_IMAGE=\n"),
    ];
    for (name, contents) in success_cases {
        let context = TestContext::new();
        setup_sse_runtime_build(&context, contents);

        let output = context.run(["backend", "build", "sse-runtime"], 0);
        assert!(output.starts_with("exit: 0\n"), "{output}");

        if context.invoke_with_bash {
            insta::assert_snapshot!(
                format!("sse_build__{name}"),
                normalize(&context.fake_tools.log(), context.root())
            );
        }
    }

    let error_cases = [
        ("key_missing", "OTHER=x\n"),
        ("key_indented", "   SSE_CONTAINER_IMAGE=img:4\n"),
    ];
    for (name, contents) in error_cases {
        let context = TestContext::new();
        setup_sse_runtime_build(&context, contents);

        let output = context.run(["backend", "build", "sse-runtime"], 1);

        insta::assert_snapshot!(format!("sse_build__{name}"), output);
    }
}

#[test]
fn need_command_reports_missing_dependencies() {
    // `curl` itself is absent from PATH.
    {
        let context = TestContext::new();
        let empty_path = context.root().join("no-tools");
        fs::create_dir_all(&empty_path).expect("create empty PATH directory");

        let output = context.run_with_env(
            ["backend", "versions", "engine"],
            1,
            [("PATH", empty_path.as_os_str())],
        );

        insta::assert_snapshot!("need_command__curl_missing", output);
    }

    // `curl` is present (the fake tool) but `jq` is not.
    {
        let context = TestContext::new();

        let output = context.run_with_env(
            ["backend", "versions", "engine"],
            1,
            [("PATH", context.fake_tools.bin_dir().as_os_str())],
        );

        insta::assert_snapshot!("need_command__jq_missing", output);
    }
}

#[test]
fn init_validates_env_name_boundaries() {
    for (snapshot_name, env_name) in [
        ("env_name__leading_hyphen", "-lead"),
        ("env_name__leading_dot", ".lead"),
        ("env_name__leading_underscore", "_lead"),
        ("env_name__uppercase", "UPPER"),
        ("env_name__space", "a b"),
        ("env_name__slash", "a/b"),
        ("env_name__single_digit", "0"),
        ("env_name__punctuation_tail", "a.b-c_d"),
    ] {
        let context = TestContext::new();
        let output = context.run(["init", env_name, "--template", "nonexistent"], 1);
        insta::assert_snapshot!(snapshot_name, output);
    }
}
