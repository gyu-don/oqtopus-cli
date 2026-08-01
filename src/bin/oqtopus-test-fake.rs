//! Multi-call fake for the external programs used by `bin/oqtopus`.
//!
//! Tests put symlinks named `curl`, `date`, `docker`, `git`, and `uv` on PATH,
//! all pointing to this executable. Behaviour is configured with a fixture
//! directory; see `tests/support/fake_tool.rs` for the test-facing API.

use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process;

const SUPPORTED_TOOLS: &[&str] = &["curl", "date", "docker", "git", "uv"];
const RECORDED_ENV_KEYS: &[&str] = &[
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "OQTOPUS_CLI_VERSION",
];

fn main() {
    if let Err(error) = run() {
        eprintln!("oqtopus-test-fake: {error}");
        process::exit(125);
    }
}

fn run() -> Result<(), String> {
    let raw_args: Vec<String> = env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let tool = invoked_tool(raw_args.first().map(String::as_str))?;
    let args = &raw_args[1..];
    let cwd = env::current_dir().map_err(|error| format!("read cwd: {error}"))?;
    let fixture_root = required_directory("OQTOPUS_TEST_FAKE_FIXTURES")?;
    let state_root = required_directory("OQTOPUS_TEST_FAKE_STATE")?;
    let sandbox = required_directory("OQTOPUS_TEST_FAKE_SANDBOX")?;
    let fixture_root = confined_target(&sandbox, &fixture_root, &sandbox)?;
    let state_root = confined_target(&sandbox, &state_root, &sandbox)?;
    let cwd = confined_target(&cwd, Path::new("."), &sandbox)?;
    let ordinal = next_ordinal(&tool, Some(&state_root))?;
    let fixture = fixture_dir(Some(&fixture_root), &tool).ok_or_else(|| {
        format!("no fixture configured for {tool} call {ordinal}; refusing to succeed implicitly")
    })?;

    if let Some(log_path) = env::var_os("OQTOPUS_TEST_FAKE_LOG") {
        let log_path = confined_target(&cwd, Path::new(&log_path), &sandbox)?;
        append_log(&log_path, &tool, args, &cwd, ordinal)?;
    }

    let stdout = read_optional(Some(&fixture), "stdout")?;
    let stderr = read_optional(Some(&fixture), "stderr")?;
    let status = read_status(Some(&fixture))?;

    if let Some(bytes) = stderr {
        io::stderr()
            .write_all(&bytes)
            .map_err(|error| format!("write stderr: {error}"))?;
    }

    if status == 0 {
        apply_default_effects(&tool, args, stdout.as_deref(), &cwd, &sandbox)?;
    }

    let suppress_stdout = tool == "curl" && curl_output_path(args).is_some();
    if let Some(bytes) = stdout.filter(|_| !suppress_stdout) {
        io::stdout()
            .write_all(&bytes)
            .map_err(|error| format!("write stdout: {error}"))?;
    }

    process::exit(status);
}

fn required_directory(name: &str) -> Result<PathBuf, String> {
    let path = env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required"))?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("{name} is not a directory: {}", path.display()))
    }
}

fn invoked_tool(argv0: Option<&str>) -> Result<String, String> {
    let executable = argv0
        .and_then(|arg| Path::new(arg).file_name())
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .trim_end_matches(".exe");
    let tool = if SUPPORTED_TOOLS.contains(&executable) {
        executable.to_owned()
    } else {
        env::var("OQTOPUS_TEST_FAKE_TOOL").unwrap_or_else(|_| executable.to_owned())
    };
    if SUPPORTED_TOOLS.contains(&tool.as_str()) {
        Ok(tool)
    } else {
        Err(format!(
            "unsupported invocation name {executable:?}; expected one of {}",
            SUPPORTED_TOOLS.join(", ")
        ))
    }
}

fn next_ordinal(tool: &str, state_root: Option<&Path>) -> Result<u64, String> {
    let Some(root) = state_root else {
        return Ok(1);
    };
    fs::create_dir_all(root).map_err(|error| format!("create state directory: {error}"))?;
    let path = root.join(format!("{tool}.count"));
    let previous = match fs::read_to_string(&path) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("parse {}: {error}", path.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let ordinal = previous + 1;
    fs::write(&path, format!("{ordinal}\n"))
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(ordinal)
}

fn fixture_dir(root: Option<&Path>, tool: &str) -> Option<PathBuf> {
    let root = root?;
    let tool_dir = root.join(tool);
    if tool_dir.is_dir() {
        Some(tool_dir)
    } else {
        None
    }
}

fn read_optional(fixture: Option<&Path>, name: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(path) = fixture.map(|dir| dir.join(name)) else {
        return Ok(None);
    };
    match fs::read(&path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("read {}: {error}", path.display())),
    }
}

fn read_status(fixture: Option<&Path>) -> Result<i32, String> {
    let Some(bytes) = read_optional(fixture, "status")? else {
        return Ok(0);
    };
    let value =
        String::from_utf8(bytes).map_err(|error| format!("status is not UTF-8: {error}"))?;
    let status = value
        .trim()
        .parse::<i32>()
        .map_err(|error| format!("invalid status {value:?}: {error}"))?;
    if (0..=255).contains(&status) {
        Ok(status)
    } else {
        Err(format!("status must be between 0 and 255, got {status}"))
    }
}

fn apply_default_effects(
    tool: &str,
    args: &[String],
    stdout: Option<&[u8]>,
    cwd: &Path,
    sandbox: &Path,
) -> Result<(), String> {
    match tool {
        "curl" => {
            if let Some(path) = curl_output_path(args) {
                let path = confined_target(cwd, &path, sandbox)?;
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("create curl output parent {}: {error}", parent.display())
                    })?;
                }
                fs::write(&path, stdout.unwrap_or_default())
                    .map_err(|error| format!("write curl output {}: {error}", path.display()))?;
            }
        }
        "git" if args.first().map(String::as_str) == Some("clone") => {
            if let Some(target) = args.last() {
                let target = confined_target(cwd, Path::new(target), sandbox)?;
                fs::create_dir_all(&target).map_err(|error| {
                    format!("create git clone target {}: {error}", target.display())
                })?;
            }
        }
        "uv" if args.first().map(String::as_str) == Some("sync") => {
            if let Some(project) = option_value(args, "--project") {
                let project = confined_target(cwd, Path::new(project), sandbox)?;
                let venv = project.join(".venv");
                fs::create_dir_all(&venv).map_err(|error| {
                    format!("create uv environment {}: {error}", venv.display())
                })?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn confined_target(cwd: &Path, requested: &Path, sandbox: &Path) -> Result<PathBuf, String> {
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        cwd.join(requested)
    };
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!(
                        "target escapes filesystem root: {}",
                        requested.display()
                    ));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
        // Resolving `..` lexically says nothing about where a symlink points, so
        // the prefix check below would accept `<sandbox>/link/out` while the write
        // lands wherever `link` points. The fakes have no reason to follow a
        // symlink, so reject one anywhere on the path rather than resolving it.
        // This also keeps the `..` handling above honest: with no symlinks on the
        // path, a lexical parent is the physical parent.
        if fs::symlink_metadata(&normalized).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(format!(
                "target traverses the symlink {}: {}",
                normalized.display(),
                requested.display()
            ));
        }
    }
    if normalized.starts_with(sandbox) {
        Ok(normalized)
    } else {
        Err(format!(
            "target is outside test sandbox {}: {}",
            sandbox.display(),
            requested.display()
        ))
    }
}

fn curl_output_path(args: &[String]) -> Option<PathBuf> {
    option_value(args, "-o")
        .or_else(|| option_value(args, "--output"))
        .map(PathBuf::from)
}

fn option_value<'a>(args: &'a [String], option: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
        .or_else(|| {
            let prefix = format!("{option}=");
            args.iter().find_map(|arg| arg.strip_prefix(&prefix))
        })
}

fn append_log(
    path: &Path,
    tool: &str,
    args: &[String],
    cwd: &Path,
    ordinal: u64,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create log directory {}: {error}", parent.display()))?;
    }
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(tool.to_owned());
    argv.extend(args.iter().cloned());

    let mut line = String::new();
    line.push_str(&format!(
        "{{\"tool\":{},\"ordinal\":{},\"argv\":[",
        json_string(tool),
        ordinal
    ));
    for (index, arg) in argv.iter().enumerate() {
        if index != 0 {
            line.push(',');
        }
        line.push_str(&json_string(arg));
    }
    line.push_str(&format!(
        "],\"cwd\":{},\"env\":{{",
        json_string(&cwd.to_string_lossy())
    ));
    for (index, key) in RECORDED_ENV_KEYS.iter().enumerate() {
        if index != 0 {
            line.push(',');
        }
        line.push_str(&json_string(key));
        line.push(':');
        match env::var_os(key) {
            Some(value) => line.push_str(&json_string(&value.to_string_lossy())),
            None => line.push_str("null"),
        }
    }
    line.push_str("}}\n");

    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open log {}: {error}", path.display()))?;
    log.write_all(line.as_bytes())
        .map_err(|error| format!("write log {}: {error}", path.display()))?;
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            value if value <= '\u{1f}' => encoded.push_str(&format!("\\u{:04x}", value as u32)),
            value => encoded.push(value),
        }
    }
    encoded.push('"');
    encoded
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{confined_target, json_string, option_value};

    #[test]
    fn json_string_escapes_control_characters() {
        assert_eq!(json_string("a\n\"\\\u{0001}"), "\"a\\n\\\"\\\\\\u0001\"");
    }

    #[test]
    fn option_value_accepts_separate_and_joined_forms() {
        let separate = vec!["--project".to_owned(), "/tmp/project".to_owned()];
        let joined = vec!["--project=/tmp/project".to_owned()];
        assert_eq!(option_value(&separate, "--project"), Some("/tmp/project"));
        assert_eq!(option_value(&joined, "--project"), Some("/tmp/project"));
    }

    #[test]
    fn confined_target_rejects_paths_outside_the_sandbox() {
        let sandbox = Path::new("/tmp/oqtopus-characterization");
        let cwd = sandbox.join("work");

        assert_eq!(
            confined_target(&cwd, Path::new("nested/output"), sandbox).unwrap(),
            sandbox.join("work/nested/output")
        );
        assert!(confined_target(&cwd, Path::new("../../outside"), sandbox).is_err());
        assert!(confined_target(&cwd, Path::new("/tmp/outside"), sandbox).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn confined_target_rejects_paths_that_traverse_a_symlink() {
        // The cases above are lexical, so they pass even without touching the
        // filesystem. A symlink escape needs real directories: the path stays
        // inside the sandbox as a string while the write lands outside it.
        let root = tempfile::tempdir().expect("create temporary directory");
        let root = std::fs::canonicalize(root.path()).expect("resolve temporary directory");
        let sandbox = root.join("sandbox");
        let cwd = sandbox.join("work");
        let outside = root.join("outside");
        std::fs::create_dir_all(&cwd).expect("create sandbox work directory");
        std::fs::create_dir_all(&outside).expect("create directory outside the sandbox");
        std::os::unix::fs::symlink(&outside, cwd.join("escape")).expect("create escaping symlink");

        // Writing *through* the symlink, and naming the symlink itself.
        assert!(confined_target(&cwd, Path::new("escape/result"), &sandbox).is_err());
        assert!(confined_target(&cwd, Path::new("escape"), &sandbox).is_err());
        // A sibling path that does not touch the symlink is still allowed.
        assert_eq!(
            confined_target(&cwd, Path::new("nested/output"), &sandbox).unwrap(),
            cwd.join("nested/output")
        );
    }
}
