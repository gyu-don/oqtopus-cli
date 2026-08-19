# Manager Environment

A manager environment is a local directory created by:

```bash
oqtopus init <env_name> --template manager
```

The template is fetched from the `main` branch of `oqtopus-cli` by default.
Pass `--branch <branch>` to fetch it from another branch instead (mainly
useful when testing in-development templates):

```bash
oqtopus init <env_name> --template manager --branch <branch>
```

Manager commands must be run from the root of this environment.

[OQTOPUS Manager](https://github.com/oqtopus-team/oqtopus-manager) is a
local/on-prem web application that lets operators manage multiple OQTOPUS
backend and cloud-local environments running on a single host from one web
UI. A manager environment is unrelated to the "environments" that OQTOPUS
Manager itself manages through its web UI; the two use the same word for
different things. See [Manager Configuration](./manager-configuration.md) for
details.

## Environment Name

`env_name` is used as the local environment directory name and is recorded in
`.metadata`.

Allowed pattern:

```text
^[a-z0-9][a-z0-9_.-]*$
```

Use lowercase letters, digits, `.`, `_`, or `-`, and start with a lowercase
letter or digit.

Examples:

- `my-manager`
- `oqtopus_manager_local`
- `manager1`

## Directory Layout

```text
<env_name>/
  .metadata
  config/
    config.yaml
    logging.yaml
  assets/
    favicon.svg
    mv_bg.png
  logs/
  pids/
```

Unlike backend and cloud-local environments, a manager environment manages a
single component and a single service, so `config/`, `logs/`, and `pids/` are
not split into per-service subdirectories.

## `.metadata`

`.metadata` records environment-specific information such as:

- the environment template (`manager`);
- the environment name;
- the absolute environment path;
- the shared manager installation root;
- the installed manager version binding (`manager_version`).

Do not move an environment directory after creating it. Manager commands check
that the current directory matches the `environment_root` recorded in `.metadata`.

## `config/`

`config/config.yaml` and `config/logging.yaml` configure the manager process.
See [Manager Configuration](./manager-configuration.md) for details, including
the `environments/` directory and `config/environments.yaml` file that
OQTOPUS Manager creates and manages on its own at runtime.

## `assets/`

`assets/` contains static branding assets (favicon, background image) referenced
by `config/config.yaml` (`appearance.app_icon_path`, `appearance.favicon_path`).

## `logs/`

`logs/` is created empty by `oqtopus init`. OQTOPUS Manager writes its own log
file here (`logs/app.log` by default) according to `config/logging.yaml`.
Because there is only one managed service, log output is not split into a
per-service subdirectory, unlike backend and cloud-local environments.

## `pids/`

`pids/` stores the PID file for the manager service (`pids/manager.pid`). The
CLI uses this file to detect the running process and to stop it safely. A
stale PID file is removed automatically when the recorded process no longer
exists.
