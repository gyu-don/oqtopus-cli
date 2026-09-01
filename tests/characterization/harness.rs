//! Shared harness for the characterization suite: sandboxed CLI invocation,
//! output rendering/normalization, environment fixtures, and builders for the
//! network payloads the fake `curl` serves.

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

use crate::fake_tool::FakeTools;

/// Asserts a snapshot under its behavior name alone, without the Rust module
/// path prefix insta would otherwise prepend. Snapshot file names therefore
/// mirror the CLI behavior they record and stay stable when tests move
/// between modules of this crate.
macro_rules! snap {
    ($name:expr, $value:expr) => {
        insta::with_settings!({prepend_module_to_snapshot => false}, {
            insta::assert_snapshot!($name, $value);
        })
    };
}
pub(crate) use snap;

const TEST_VERSION: &str = "9.8.7-characterization";

/// Terminates every snapshot. insta trims trailing whitespace, so without a
/// closing line the final section's trailing newline would go unrecorded.
const END_SENTINEL: &str = "--- end ---\n";

pub(crate) struct TestContext {
    binary: PathBuf,
    pub(crate) invoke_with_bash: bool,
    // Owns the temporary directory; every path is taken from `root` instead.
    _sandbox: TempDir,
    root: PathBuf,
    pub(crate) fake_tools: FakeTools,
}

impl TestContext {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn run<I, S>(&self, args: I, expected_code: i32) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with_env(args, expected_code, std::iter::empty::<(&str, &str)>())
    }

    pub(crate) fn run_with_env<I, S, E, K, V>(
        &self,
        args: I,
        expected_code: i32,
        extra_env: E,
    ) -> String
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

    pub(crate) fn work_dir(&self) -> PathBuf {
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

pub(crate) fn normalize(value: &str, sandbox: &Path) -> String {
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

pub(crate) fn render_observation(
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
pub(crate) struct EnvFixture {
    pub(crate) install_root: PathBuf,
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
    pub(crate) fn backend(context: &TestContext, extra: &[(&str, &str)]) -> Self {
        Self::create(context, "backend", "backend", extra)
    }

    /// Writes `.metadata` for a `manager` environment into the work directory.
    pub(crate) fn manager(context: &TestContext, extra: &[(&str, &str)]) -> Self {
        Self::create(context, "manager", "manager", extra)
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
    pub(crate) fn create_legacy(
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
    pub(crate) fn install_release(&self, name: &str) -> PathBuf {
        let path = self.install_root.join(name);
        fs::create_dir_all(&path)
            .unwrap_or_else(|error| panic!("create release {}: {error}", path.display()));
        path
    }
}

/// Packs `<root>/archive-source/<top_level>` (which `populate` must fill in)
/// into a gzipped tarball, as GitHub's codeload archives are shaped: a single
/// top-level directory wrapping the repository contents.
pub(crate) fn build_targz(root: &Path, top_level: &str, populate: impl FnOnce(&Path)) -> Vec<u8> {
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

pub(crate) fn build_backend_template_archive(root: &Path) -> Vec<u8> {
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
pub(crate) fn advertised_refs(refs: &[&str]) -> Vec<u8> {
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
pub(crate) fn advertised_tags(tags: &[&str]) -> Vec<u8> {
    let refs: Vec<String> = tags.iter().map(|tag| format!("tags/{tag}")).collect();
    advertised_refs(&refs.iter().map(String::as_str).collect::<Vec<_>>())
}
