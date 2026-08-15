# Starting And Stopping The Manager Service

This page describes service lifecycle for `--template manager` environments.
For backend service management, see [Starting and Stopping Backend Services](./backend-lifecycle.md).
For cloud-local service management, see [Starting and Stopping Cloud-Local Services](./cloud-local-lifecycle.md).

OQTOPUS CLI can start, stop, restart, and inspect the manager service from
inside a manager environment.

## Managed Service

Unlike backend and cloud-local, manager has a single managed service and no
service name argument for `start`, `stop`, `restart`, or `status`.

## Start

```bash
oqtopus manager start
```

If the service is already running, `start` fails instead of starting a
duplicate process.

Started services are detached from the short-lived CLI process and continue
running after `oqtopus manager start` exits.

For debugging, start in foreground mode:

```bash
oqtopus manager start --foreground
```

Foreground mode keeps the service attached to the terminal, so runtime stdout
and stderr are visible. The command exits when the service process exits.

## Check Process Status

```bash
oqtopus manager status
```

Example output:

```text
manager: Running (PID 12345)
```

or:

```text
manager: Stopped
```

If a PID file exists but the process is no longer alive, the service is
treated as stopped.

## Stop

```bash
oqtopus manager stop
```

If no PID file exists, the service is treated as already stopped.

`stop` sends `TERM` and waits up to 5 seconds. It does not send `KILL`
automatically.

## Restart

```bash
oqtopus manager restart
```

`restart` stops the service and starts it again. If `stop` fails, the service
is not started again.

## Process Output And Logs

By default, runtime stdout and stderr are redirected to `/dev/null`.

Use `oqtopus manager start --foreground` when you need to inspect runtime
stdout and stderr directly while debugging.

OQTOPUS CLI does not create application log files. OQTOPUS Manager writes its
own log file according to `config/logging.yaml` (`logs/app.log` by default,
relative to the environment root).
