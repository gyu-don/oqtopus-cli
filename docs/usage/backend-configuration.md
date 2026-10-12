# Configuration

This page describes configuration for `--template backend` environments. For
cloud-local configuration, see [Cloud-Local Configuration](./cloud-local-configuration.md).

This page will describe how to configure an OQTOPUS backend environment.

Configuration files are created under `config/` when you run:

```bash
oqtopus init <env_name> --template backend
```

The generated files are examples for the local backend environment. Review and
update them as needed before starting services.

## Configuration Directory

```text
config/
  .env
  core/
  sse_engine/
  mitigator/
  estimator/
  combiner/
  tranqu/
  gateway/
```

## Environment Variables

`config/.env` contains environment variables used when launching backend
processes.

These values are applied only to the launched process environment. They are not
persisted to your global shell environment.

`config/.env` is a configuration file and should not contain secrets.

## Environment Name Placeholders

The backend template may use `{{ env_name }}` in `config/.env`.

When you run:

```bash
oqtopus init my-demo --template backend
```

OQTOPUS CLI replaces the placeholder with the validated environment name.

For example:

```env
SSE_CONTAINER_IMAGE="{{ env_name }}-sse_runtime:latest"
```

becomes:

```env
SSE_CONTAINER_IMAGE="my-demo-sse_runtime:latest"
```

These values are used by Docker, so `env_name` must be Docker-safe:

```text
^[a-z0-9][a-z0-9_.-]*$
```

## SSE Network Settings

SSE jobs run user programs in an `sse_runtime` Docker container. The container
connects to `sse_engine`, which runs as a host process, over gRPC:

```text
sse_runtime container --(sse_engine:51014)--> docker0 (172.17.0.1) --> sse_engine (host)
```

Two variables in `config/.env` control this, and they have different roles:

| Variable | Default | Role |
|---|---|---|
| `SSE_ENGINE_ADDRESS` | `"sse_engine:51014"` | Address the container connects to. `sse_engine` is resolved inside the container through `container_extra_hosts` (`host-gateway`). |
| `SSE_ENGINE_BIND_ADDRESS` | `"172.17.0.1:51014"` | Address `sse_engine` listens on, in `host:port` form. The port is required. |

`SSE_ENGINE_BIND_ADDRESS` must be an address the container can reach, i.e. the
IP of `docker0`, which is also what `host-gateway` resolves to. The default
`172.17.0.1` is the usual value on Linux. If your `docker0` uses a different IP
(for example after changing `bip` or `default-address-pools` in Docker's
`daemon.json`), check it and update the variable:

```bash
ip -4 addr show docker0 | grep inet
```

Notes:

- `127.0.0.1` is not reachable from the container, so SSE jobs fail.
- `0.0.0.0` works but listens on all interfaces. Avoid it on shared machines.
- Docker must be running before `sse_engine` starts. Otherwise `docker0` does
  not exist and binding fails.
- Docker Desktop (Mac / Windows, including WSL2 integration) is not supported,
  because `docker0` is not present on the host side. Use Docker Engine
  installed directly on Linux (on Windows, inside a WSL2 distribution without
  Docker Desktop's WSL integration).
- Rootless Docker is supported only if you configure the network yourself. Its
  `host-gateway` target differs from `docker0`, so set `SSE_ENGINE_BIND_ADDRESS`
  and `SSE_ENGINE_IP` (an environment variable read by `config/core/config.yaml`,
  not listed in `.env`) to an address reachable from the container.

## Service Configuration

Each service has its own configuration directory under `config/`.

Detailed configuration guidance will be added as the supported options are
documented.
