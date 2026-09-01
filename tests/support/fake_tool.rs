//! Test-facing setup and fixture helpers for `oqtopus-test-fake`.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

const TOOLS: &[&str] = &["curl", "date", "docker", "uv"];

/// An isolated PATH directory, fixture tree, state tree, and JSONL invocation log.
pub struct FakeTools {
    bin: PathBuf,
    fixtures: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

impl FakeTools {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().join("fake-tools");
        let fake = Self {
            bin: root.join("bin"),
            fixtures: root.join("fixtures"),
            state: root.join("state"),
            log: root.join("calls.jsonl"),
        };
        fake.install();
        fake
    }

    fn install(&self) {
        fs::create_dir_all(&self.bin).expect("create fake tool bin directory");
        fs::create_dir_all(&self.fixtures).expect("create fake tool fixture directory");
        fs::create_dir_all(&self.state).expect("create fake tool state directory");
        let executable = Path::new(env!("CARGO_BIN_EXE_oqtopus-test-fake"));
        for tool in TOOLS {
            let link = self.bin.join(tool);
            #[cfg(unix)]
            std::os::unix::fs::symlink(executable, &link)
                .unwrap_or_else(|error| panic!("link fake tool {}: {error}", link.display()));
            #[cfg(not(unix))]
            fs::copy(executable, &link)
                .unwrap_or_else(|error| panic!("copy fake tool {}: {error}", link.display()));
        }
    }

    /// The directory holding the fake executables. Usable as a PATH on its own
    /// when a test needs every other command to be absent.
    pub fn bin_dir(&self) -> &Path {
        &self.bin
    }

    /// Prefixes the fake tools to an existing PATH value.
    pub fn path_with(&self, existing: impl AsRef<OsStr>) -> OsString {
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(existing.as_ref()));
        std::env::join_paths(paths).expect("construct PATH containing fake tools")
    }

    /// Values to add to the environment of the Bash CLI under test.
    pub fn env(&self) -> [(&'static str, &OsStr); 4] {
        [
            ("OQTOPUS_TEST_FAKE_LOG", self.log.as_os_str()),
            ("OQTOPUS_TEST_FAKE_FIXTURES", self.fixtures.as_os_str()),
            ("OQTOPUS_TEST_FAKE_STATE", self.state.as_os_str()),
            (
                "OQTOPUS_TEST_FAKE_SANDBOX",
                self.bin
                    .parent()
                    .and_then(Path::parent)
                    .expect("fake tools live below the test sandbox")
                    .as_os_str(),
            ),
        ]
    }

    /// Configures the fallback response for every call to `tool`.
    pub fn fixture(&self, tool: &str) -> Fixture {
        self.assert_tool(tool);
        Fixture::new(self.fixtures.join(tool))
    }

    /// Configures the response for the `ordinal`-th call to `tool` (1-based).
    /// A per-call response takes precedence over the tool's fallback response,
    /// so a test can give curl a ref advertisement on the first call and a
    /// tarball on the second.
    pub fn fixture_call(&self, tool: &str, ordinal: u64) -> Fixture {
        self.assert_tool(tool);
        Fixture::new(self.fixtures.join(tool).join(format!("call-{ordinal}")))
    }

    pub fn log(&self) -> String {
        match fs::read_to_string(&self.log) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("read fake tool log {}: {error}", self.log.display()),
        }
    }

    pub fn call_count(&self, tool: &str) -> u64 {
        self.assert_tool(tool);
        let path = self.state.join(format!("{tool}.count"));
        match fs::read_to_string(&path) {
            Ok(value) => value.trim().parse().unwrap_or_else(|error| {
                panic!("parse fake tool count {}: {error}", path.display())
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("read fake tool count {}: {error}", path.display()),
        }
    }

    fn assert_tool(&self, tool: &str) {
        assert!(TOOLS.contains(&tool), "unsupported fake tool: {tool}");
    }
}

/// Builder for one tool response. Files are written eagerly to keep setup simple.
pub struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(path: PathBuf) -> Self {
        fs::create_dir_all(&path).expect("create fake tool fixture");
        Self { path }
    }

    pub fn stdout(self, value: impl AsRef<[u8]>) -> Self {
        self.write("stdout", value);
        self
    }

    fn write(&self, name: &str, value: impl AsRef<[u8]>) {
        let path = self.path.join(name);
        fs::write(&path, value)
            .unwrap_or_else(|error| panic!("write fake tool fixture {}: {error}", path.display()));
    }
}
