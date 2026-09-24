"""Exercise production history acquisition against a local HTTPS Git server."""

import base64
import contextlib
import hashlib
import http.server
import json
import os
import shutil
import ssl
import subprocess
import sys
import tempfile
import threading
import urllib.parse
from pathlib import Path

TOKEN = "history-action-fixture"
CHECKOUT = (
    "Basic " + base64.b64encode(b"x-access-token:history-checkout-fixture").decode()
)
AUTHORIZATION = "basic " + base64.b64encode(f"x-access-token:{TOKEN}".encode()).decode()


def run(arguments, *, cwd, env, check=True):
    result = subprocess.run(
        arguments, cwd=cwd, env=env, capture_output=True, timeout=60, check=False
    )
    if check and result.returncode:
        raise AssertionError(
            f"{arguments[0]} failed ({result.returncode}):\n{result.stdout.decode()}\n{result.stderr.decode()}"
        )
    return result


def git(root, env, *args):
    return run(["git", *args], cwd=root, env=env).stdout.decode().strip()


def install_source_fixture(root, env, executable, core):
    target = next(
        line.removeprefix("host: ")
        for line in run(["rustc", "-vV"], cwd=root, env=env)
        .stdout.decode()
        .splitlines()
        if line.startswith("host: ")
    )
    version = (
        run([str(core), "rail", "--version"], cwd=root, env=env)
        .stdout.decode()
        .strip()
        .removeprefix("cargo-rail ")
    )
    license_path = Path(__file__).resolve().parents[2] / "LICENSE"
    suffix = ".exe" if os.name == "nt" else ""
    rows = ""
    for name, path, capability in [
        (f"cargo-rail{suffix}", core, "core"),
        ("LICENSE", license_path, "license"),
    ]:
        contents = path.read_bytes()
        rows += f"{name}\t{hashlib.sha256(contents).hexdigest()}\t{len(contents)}\t{capability}\n"
    digest = hashlib.sha256(
        f"cargo-rail-components-v1\t{version}\t{target}\n{rows}".encode()
    ).hexdigest()
    installation = (
        root
        / "cache"
        / "cargo-rail-action"
        / "cargo-rail"
        / version
        / target
        / f"core-{digest}"
    )
    installation.mkdir(parents=True)
    shutil.copy2(core, installation / f"cargo-rail{suffix}")
    shutil.copy2(license_path, installation / "LICENSE")
    (installation / "cargo-rail-action-install-v1.tsv").write_bytes(
        f"cargo-rail-action-installed-v1\t{version}\t{target}\tcore\t{digest}\n{rows}".encode()
    )
    runtime = (
        root / f"{target}-{hashlib.sha256(Path(executable).read_bytes()).hexdigest()}"
    )
    runtime.mkdir()
    shutil.copy2(license_path, runtime / "LICENSE")
    installed_runtime = runtime / f"cargo-rail-action-{target}{suffix}"
    shutil.copy2(executable, installed_runtime)
    return str(installed_runtime), version


class GitServer(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        self.serve_git()

    def do_POST(self):
        self.serve_git()

    def serve_git(self):
        authorization = self.headers.get_all("Authorization", [])
        self.server.requests.append(authorization)
        if authorization != [self.server.authorization]:
            self.send_error(400, "expected one authoritative Authorization header")
            return
        if self.server.failure_status:
            self.send_error(self.server.failure_status, f"fixture failure {TOKEN}")
            return
        if self.server.fail_first and len(self.server.requests) == 1:
            self.send_error(503, "fixture rejects the first SHA fetch")
            return
        url = urllib.parse.urlsplit(self.path)
        env = dict(self.server.environment)
        env.update(
            GIT_PROJECT_ROOT=str(self.server.root),
            GIT_HTTP_EXPORT_ALL="1",
            REQUEST_METHOD=self.command,
            PATH_INFO=url.path.replace("/repo/", "/repo.git/"),
            QUERY_STRING=url.query,
            CONTENT_TYPE=self.headers.get("Content-Type", ""),
            CONTENT_LENGTH=self.headers.get("Content-Length", "0"),
        )
        body = self.rfile.read(int(env["CONTENT_LENGTH"]))
        response = subprocess.run(
            ["git", "http-backend"],
            input=body,
            env=env,
            capture_output=True,
            timeout=30,
            check=True,
        ).stdout
        headers, body = response.split(b"\r\n\r\n", 1)
        fields = [line.decode().split(": ", 1) for line in headers.split(b"\r\n")]
        status = next(
            (int(value.split()[0]) for key, value in fields if key == "Status"), 200
        )
        self.send_response(status)
        for key, value in fields:
            if key != "Status":
                self.send_header(key, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@contextlib.contextmanager
def fixture():
    with tempfile.TemporaryDirectory(prefix="action-history-") as temporary:
        root = Path(temporary).resolve()
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(
                ("GIT_", "GITHUB_", "INPUT_", "HISTORY_FIXTURE_", "CARGO_RAIL_")
            )
            and key
            not in ("CARGO_TARGET_DIR", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")
        }
        env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_TERMINAL_PROMPT="0",
            NO_PROXY="localhost,127.0.0.1",
        )
        env.update(
            GIT_AUTHOR_NAME="History Fixture",
            GIT_AUTHOR_EMAIL="history@example.invalid",
            GIT_COMMITTER_NAME="History Fixture",
            GIT_COMMITTER_EMAIL="history@example.invalid",
        )
        cert, key = root / "cert.pem", root / "key.pem"
        run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                str(key),
                "-out",
                str(cert),
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
            ],
            cwd=root,
            env=env,
        )
        seed = root / "seed"
        git(root, env, "init", "--initial-branch=main", str(seed))
        (seed / "Cargo.toml").write_text(
            "[package]\nname='history-fixture'\nversion='0.1.0'\nedition='2024'\n[lib]\npath='lib.rs'\n"
        )
        (seed / "lib.rs").write_text("pub fn value() -> u8 { 1 }\n")
        (seed / ".gitignore").write_text("target/\n")
        run(["cargo", "generate-lockfile", "--offline"], cwd=seed, env=env)
        git(seed, env, "add", ".")
        git(seed, env, "commit", "-m", "initial")
        ancestor = git(seed, env, "rev-parse", "HEAD")
        git(seed, env, "branch", "topic")
        (seed / "main.txt").write_text("base branch\n")
        git(seed, env, "add", ".")
        git(seed, env, "commit", "-m", "base")
        base = git(seed, env, "rev-parse", "HEAD")
        git(seed, env, "tag", "baseline")
        git(seed, env, "checkout", "topic")
        (seed / "lib.rs").write_text("pub fn value() -> u8 { 2 }\n")
        git(seed, env, "add", ".")
        git(seed, env, "commit", "-m", "topic")
        git(seed, env, "checkout", "-b", "synthetic-merge", "main")
        git(
            seed, env, "merge", "--no-ff", "topic", "-m", "synthetic pull request merge"
        )
        remote = root / "owner" / "repo.git"
        remote.parent.mkdir()
        git(root, env, "clone", "--bare", str(seed), str(remote))
        server = http.server.ThreadingHTTPServer(("localhost", 0), GitServer)
        server.root, server.environment = root, env
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield root, env, cert, remote, server, base, ancestor
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


def main():
    executable = str(Path(sys.argv[1]).resolve())
    core = Path(sys.argv[2]).resolve() if len(sys.argv) == 3 else None
    failures = []
    with fixture() as (root, env, cert, remote, server, base, ancestor):
        if core:
            executable, version = install_source_fixture(root, env, executable, core)
        authority = f"https://localhost:{server.server_port}"
        cases = [
            ("pull-request", "", base),
            ("empty", "HEAD", None),
            ("forced", "", None),
            ("widened", "HEAD", None),
            ("sha", base, base),
            ("sha-retry", base, base),
            ("remote-branch", "origin/main", base),
            ("local-branch", "refs/heads/main", base),
            ("tag", "refs/tags/baseline", base),
            ("bare-branch", "main", base),
            ("bare-tag", "baseline", base),
            ("head-relative", "HEAD~2", ancestor),
            ("checkout-only", "", base),
            ("token-only", "", base),
            ("wrong-repository", "", base),
            ("wrong-server", "", base),
            ("denied", "refs/heads/main", base),
            ("denied-checkout", "refs/heads/main", base),
            ("unavailable", "refs/heads/main", base),
            ("denied-bare", "main", base),
            ("unavailable-bare", "main", base),
        ]
        for suffix in ("", ".git"):
            for name, since, expected in cases:
                case = root / f"{name}{suffix}"
                git(
                    root,
                    env,
                    "clone",
                    "--depth=1",
                    "--no-tags",
                    "--branch=synthetic-merge",
                    remote.as_uri(),
                    str(case),
                )
                git(case, env, "checkout", "--detach")
                if expected is None:
                    expected = git(case, env, "rev-parse", "HEAD")
                origin = f"{authority}/owner/repo{suffix}"
                git(case, env, "remote", "set-url", "origin", origin)
                git(case, env, "config", "http.sslCAInfo", str(cert))
                if os.name == "nt":
                    git(case, env, "config", "http.schannelUseSSLCAInfo", "true")
                # Checkout can persist credentials in an included file rather than .git/config.
                credentials = root / f"{name}{suffix}.config"
                credentials.touch()
                if name != "token-only":
                    git(
                        root,
                        env,
                        "config",
                        "--file",
                        str(credentials),
                        f"http.{authority}/.extraHeader",
                        f"Authorization: {CHECKOUT}",
                    )
                git(case, env, "config", "include.path", str(credentials))
                if name not in (
                    "pull-request",
                    "checkout-only",
                    "denied-checkout",
                    "token-only",
                ):
                    git(
                        case,
                        env,
                        "config",
                        "--add",
                        f"http.{origin}/.extraHeader",
                        f"Authorization: {CHECKOUT}",
                    )
                config_before = (case / ".git" / "config").read_bytes()
                credentials_before = credentials.read_bytes()
                if name == "widened":
                    (case / ".config").mkdir()
                    (case / ".config" / "nextest.toml").write_text(
                        "[profile.default]\nfail-fast = true\n"
                    )
                server.requests = []
                server.authorization = (
                    CHECKOUT
                    if name in ("checkout-only", "denied-checkout")
                    else AUTHORIZATION
                )
                server.fail_first = name == "sha-retry"
                server.failure_status = {"denied": 401, "unavailable": 503}.get(
                    name.removesuffix("-bare").removesuffix("-checkout")
                )
                child_env = dict(
                    env,
                    GITHUB_SERVER_URL=authority,
                    GITHUB_REPOSITORY="owner/repo",
                    GITHUB_EVENT_NAME="pull_request",
                    GITHUB_BASE_REF="main",
                    INPUT_SINCE=since,
                    INPUT_ALL="true" if name == "forced" else "false",
                    INPUT_REPOSITORY_TOKEN=""
                    if name in ("checkout-only", "denied-checkout")
                    else TOKEN,
                    HISTORY_FIXTURE_WORKSPACE=str(case),
                    HISTORY_FIXTURE_BASE=expected,
                )
                rejected = name.startswith("wrong-")
                fetch_expected = not rejected and name not in (
                    "empty",
                    "forced",
                    "widened",
                )
                cause = {"denied": "authentication", "unavailable": "unavailable"}.get(
                    name.removesuffix("-bare").removesuffix("-checkout")
                )
                if cause:
                    child_env["HISTORY_FIXTURE_CAUSE"] = cause
                    child_env["HISTORY_FIXTURE_RECOVERY"] = (
                        (
                            "checkout credentials"
                            if name == "denied-checkout"
                            else "repository-token access"
                        )
                        if cause == "authentication"
                        else "Retry after the remote Git service"
                    )
                if rejected:
                    child_env["HISTORY_FIXTURE_REJECT"] = "true"
                    if name == "wrong-repository":
                        child_env["GITHUB_REPOSITORY"] = "owner/elsewhere"
                    else:
                        child_env["GITHUB_SERVER_URL"] = "https://elsewhere.invalid"
                if core:
                    runner = root / f"runner-{name}{suffix}"
                    runner.mkdir()
                    for file in ("output", "path", "summary"):
                        (runner / file).touch()
                    child_env.update(
                        RUNNER_TEMP=str(runner),
                        RUNNER_TOOL_CACHE=str(root / "cache"),
                        GITHUB_WORKSPACE=str(case),
                        GITHUB_OUTPUT=str(runner / "output"),
                        GITHUB_PATH=str(runner / "path"),
                        GITHUB_STEP_SUMMARY=str(runner / "summary"),
                        INPUT_VERSION=version,
                        INPUT_COMPONENTS="core",
                        INPUT_WORKING_DIRECTORY=".",
                        INPUT_EVIDENCE="",
                    )
                    invocation = [executable, "run", "planner"]
                else:
                    invocation = [
                        executable,
                        "--exact",
                        "repository::tests::history_fetch_uses_one_authentication_authority",
                        "--nocapture",
                    ]
                result = run(invocation, cwd=case, env=child_env, check=False)
                expected_exit = (
                    (2 if rejected else 1) if core and (rejected or cause) else 0
                )
                bad_headers = [
                    len(headers)
                    for headers in server.requests
                    if headers != [server.authorization]
                ]
                if (
                    result.returncode != expected_exit
                    or bad_headers
                    or (bool(server.requests) != fetch_expected)
                ):
                    failures.append(
                        f"{name}{suffix}: exit={result.returncode}, requests={len(server.requests)}, invalid header counts={bad_headers}\n{result.stdout.decode()}\n{result.stderr.decode()}"
                    )
                elif cause and len(server.requests) != 1:
                    failures.append(
                        f"{name}{suffix}: expected one failed fetch, got {len(server.requests)}"
                    )
                elif core:
                    outputs = (runner / "output").read_text()
                    if rejected:
                        assert not outputs and not result.stdout
                        if b"origin disagrees" not in result.stderr:
                            failures.append(
                                f"{name}{suffix}: missing origin rejection:\n{result.stderr.decode()}"
                            )
                    elif cause:
                        assert not outputs and not result.stdout
                        assert cause.encode() in result.stderr
                        assert result.stderr.count(b"Next: ") == 1
                    else:
                        plans = list(runner.glob("cargo-rail-plan.*/plan.json"))
                        assert len(plans) == 1, outputs
                        plan = json.loads(plans[0].read_text())
                        assert plan["inputs"]["base"] == expected, plan["inputs"]
                        assert str(plans[0]) in outputs, outputs
                        assert b"Cargo-Rail plan ready:" in result.stdout
                        summary = (runner / "summary").read_text()
                        assert "Cargo-Rail" in summary
                        if name == "empty":
                            assert not plan["required"], plan["required"]
                            assert "No work required." in summary, summary
                        elif name == "forced":
                            assert plan["required"], plan["required"]
                            assert all(
                                work["cause"] == "forced_all"
                                for work in plan["work"].values()
                            ), plan["work"]
                            assert "Required by `--all`." in summary, summary
                        elif name == "widened":
                            assert (
                                plan["work"]["cargo.test"]["cause"]
                                == "incomplete_evidence"
                            ), plan["work"]["cargo.test"]
                            assert "Scope expanded:" in summary, summary
                        elif name == "pull-request":
                            assert (
                                plan["work"]["cargo.test"]["cause"] == "changed_input"
                            ), plan["work"]["cargo.test"]
                            assert "history-fixture" in summary, summary
                        selected = run(
                            [executable, "plan", "required", str(plans[0])],
                            cwd=case,
                            env=dict(
                                child_env,
                                PATH=str(core.parent) + os.pathsep + env["PATH"],
                            ),
                        )
                        if name == "empty":
                            assert json.loads(selected.stdout) == [], plan
                        else:
                            assert "cargo.test" in json.loads(selected.stdout), plan
                    for credential in (
                        TOKEN,
                        "history-checkout-fixture",
                        CHECKOUT.split()[-1],
                        AUTHORIZATION.split()[-1],
                    ):
                        assert (
                            credential
                            not in outputs
                            + result.stdout.decode()
                            + result.stderr.decode()
                            + (runner / "summary").read_text()
                        )
                assert (case / ".git" / "config").read_bytes() == config_before
                assert credentials.read_bytes() == credentials_before
        assert not failures, "\n".join(failures)
        print(f"{len(cases) * 2} HTTPS history cases passed")


if __name__ == "__main__":
    main()
