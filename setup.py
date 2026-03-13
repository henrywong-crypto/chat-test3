#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "requests",
#   "rich",
# ]
# ///
"""
WebCode setup script.

Downloads Firecracker, builds the project, installs binaries, builds the VM
kernel + rootfs, and writes config.toml.  Run from the repository root.

Usage:
    ./setup.py                          # full setup
    ./setup.py --skip-rootfs            # skip the slow rootfs build
    ./setup.py --skip-firecracker       # skip downloading Firecracker binaries
    ./setup.py --skip-build             # skip cargo build (assumes already built)
    ./setup.py --skip-db                # skip PostgreSQL setup
"""

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

import requests
from rich.console import Console
from rich.panel import Panel
from rich.progress import BarColumn, DownloadColumn, Progress, TransferSpeedColumn

console = Console()

# ── Versions ──────────────────────────────────────────────────────────────────
FC_VERSION = "1.10.1"       # Firecracker binary release from GitHub
FC_CI_VERSION = "v1.14"     # S3 CI bucket version for kernel + rootfs images
ARCH = "x86_64"

# ── Paths ─────────────────────────────────────────────────────────────────────
FC_DIR = Path("/var/lib/fc")
USER_ROOTFS_DIR = Path("/home/ubuntu/fc-users")
JAILER_CHROOT_BASE = Path("/srv/jailer")
USR_LOCAL_BIN = Path("/usr/local/bin")

KERNEL_DEST = FC_DIR / "vmlinux"
ROOTFS_DEST = FC_DIR / "rootfs.ext4"
SSH_KEY = FC_DIR / "id_rsa"
SSH_KEY_PUB = FC_DIR / "id_rsa.pub"
VM_HOST_KEY = FC_DIR / "vm_host_ed25519_key"
VM_HOST_KEY_PUB = FC_DIR / "vm_host_ed25519_key.pub"

# ── Output helpers ─────────────────────────────────────────────────────────────
def step(title: str) -> None:
    console.print(f"\n[bold cyan]▶  {title}[/bold cyan]")

def ok(msg: str) -> None:
    console.print(f"   [green]✓[/green]  {msg}")

def skip(msg: str) -> None:
    console.print(f"   [yellow]–[/yellow]  {msg}  [dim](skipping, already done)[/dim]")

def info(msg: str) -> None:
    console.print(f"   [dim]{msg}[/dim]")

def die(msg: str) -> None:
    console.print(f"\n[bold red]ERROR:[/bold red] {msg}")
    sys.exit(1)

# ── Subprocess helpers ─────────────────────────────────────────────────────────
def run(
    cmd: list[str],
    cwd: Path | None = None,
    env: dict | None = None,
    check: bool = True,
    capture: bool = False,
) -> subprocess.CompletedProcess:
    merged = {**os.environ, **(env or {})}
    return subprocess.run(
        cmd,
        cwd=cwd,
        env=merged,
        check=check,
        capture_output=capture,
        text=capture,
    )

def sudo(*cmd: str, cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return run(["sudo", *cmd], cwd=cwd, check=check)

def sudo_write(path: Path, content: str) -> None:
    """Write content to a privileged path via tee."""
    proc = subprocess.Popen(
        ["sudo", "tee", str(path)],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
    )
    proc.communicate(content.encode())
    if proc.returncode != 0:
        die(f"sudo tee {path} failed")

# ── Download helper ────────────────────────────────────────────────────────────
def download(url: str, dest: Path) -> None:
    info(f"← {url}")
    response = requests.get(url, stream=True, timeout=300)
    if not response.ok:
        die(f"Download failed ({response.status_code}): {url}")
    total = int(response.headers.get("content-length", 0))
    with Progress(
        "   [progress.description]{task.description}",
        BarColumn(),
        DownloadColumn(),
        TransferSpeedColumn(),
        console=console,
        transient=True,
    ) as progress:
        task = progress.add_task(dest.name, total=total or None)
        with dest.open("wb") as fh:
            for chunk in response.iter_content(chunk_size=65536):
                fh.write(chunk)
                progress.update(task, advance=len(chunk))
    ok(f"→ {dest}")

# ══════════════════════════════════════════════════════════════════════════════
# Step 1 — Prerequisites
# ══════════════════════════════════════════════════════════════════════════════
def check_prerequisites() -> None:
    step("Checking prerequisites")

    required = {
        "cargo":       "Rust — https://rustup.rs",
        "ssh-keygen":  "openssh-client — sudo apt install openssh-client",
        "setcap":      "libcap2-bin — sudo apt install libcap2-bin",
        "unsquashfs":  "squashfs-tools — sudo apt install squashfs-tools",
        "mkfs.ext4":   "e2fsprogs — sudo apt install e2fsprogs",
        "truncate":    "coreutils",
        "node":        "Node.js — sudo apt install nodejs",
        "npm":         "npm — sudo apt install npm",
    }
    missing = []
    for binary, hint in required.items():
        if shutil.which(binary):
            ok(binary)
        else:
            console.print(f"   [red]✗[/red]  {binary}  [dim]({hint})[/dim]")
            missing.append(binary)
    if missing:
        die(f"Missing tools: {', '.join(missing)}")

    result = run(["pg_isready"], check=False, capture=True)
    if result.returncode == 0:
        ok("postgresql")
    else:
        die("PostgreSQL is not running — sudo systemctl start postgresql")

    run(["sudo", "-v"])
    ok("sudo credentials refreshed")

# ══════════════════════════════════════════════════════════════════════════════
# Step 2 — Directories
# ══════════════════════════════════════════════════════════════════════════════
def setup_directories() -> None:
    step("Creating directories")
    for d in [FC_DIR, USER_ROOTFS_DIR, JAILER_CHROOT_BASE]:
        if d.exists():
            skip(str(d))
        else:
            sudo("mkdir", "-p", str(d))
            ok(f"Created {d}")
    current_user = os.environ.get("USER", "ubuntu")
    sudo("chown", f"{current_user}:{current_user}", str(USER_ROOTFS_DIR), check=False)

# ══════════════════════════════════════════════════════════════════════════════
# Step 3 — Firecracker + jailer binaries
# ══════════════════════════════════════════════════════════════════════════════
def install_firecracker(fc_version: str) -> None:
    step(f"Installing Firecracker v{fc_version} + jailer")

    fc_dest = USR_LOCAL_BIN / "firecracker"
    jailer_dest = USR_LOCAL_BIN / "jailer"
    if fc_dest.exists() and jailer_dest.exists():
        skip("firecracker + jailer")
        return

    url = (
        f"https://github.com/firecracker-microvm/firecracker/releases/download/"
        f"v{fc_version}/firecracker-v{fc_version}-{ARCH}.tgz"
    )

    with tempfile.TemporaryDirectory() as tmp_str:
        tmp = Path(tmp_str)
        archive = tmp / "firecracker.tgz"
        download(url, archive)

        with tarfile.open(archive) as tf:
            tf.extractall(tmp)

        # Binaries sit in release-v{ver}-{arch}/
        fc_src = next(
            (p for p in tmp.rglob(f"firecracker-v{fc_version}-{ARCH}")
             if not p.name.endswith(".debug")),
            None,
        )
        jailer_src = next(
            (p for p in tmp.rglob(f"jailer-v{fc_version}-{ARCH}")
             if not p.name.endswith(".debug")),
            None,
        )
        if not fc_src or not jailer_src:
            die(f"Could not find binaries inside {archive}")

        sudo("install", "-o", "root", "-g", "root", "-m", "0755", str(fc_src), str(fc_dest))
        # jailer needs setuid root
        sudo("install", "-o", "root", "-g", "root", "-m", "4755", str(jailer_src), str(jailer_dest))

    ok(f"Installed {fc_dest}")
    ok(f"Installed {jailer_dest}  (setuid root)")

# ══════════════════════════════════════════════════════════════════════════════
# Step 4 — PostgreSQL database
# ══════════════════════════════════════════════════════════════════════════════
def setup_database(database_url: str) -> None:
    step("Setting up PostgreSQL")

    result = run(
        ["sudo", "-u", "postgres", "psql", "-lqt"],
        check=False, capture=True,
    )
    if "webcode" in result.stdout:
        skip("database 'webcode'")
        return

    run(["sudo", "-u", "postgres", "createdb", "webcode"])
    ok("Created database 'webcode'")

    current_user = os.environ.get("USER", "ubuntu")
    run(
        ["sudo", "-u", "postgres", "psql", "-c",
         f"GRANT ALL PRIVILEGES ON DATABASE webcode TO \"{current_user}\";"],
        check=False,
    )
    ok(f"Granted access to '{current_user}'")

# ══════════════════════════════════════════════════════════════════════════════
# Step 5 — Build the Rust project
# ══════════════════════════════════════════════════════════════════════════════
def build_project(project_dir: Path, database_url: str) -> None:
    step("Building project  (cargo build --release)")
    info("This may take a few minutes on first build…")

    env = {"DATABASE_URL": database_url}
    result = run(
        ["cargo", "build", "--release"],
        cwd=project_dir,
        env=env,
        check=False,
    )
    if result.returncode != 0:
        info("Live DB check failed — retrying with SQLX_OFFLINE=true")
        result = run(
            ["cargo", "build", "--release"],
            cwd=project_dir,
            env={"SQLX_OFFLINE": "true"},
            check=False,
        )
    if result.returncode != 0:
        die("cargo build --release failed — check output above")

    ok("Build complete")

# ══════════════════════════════════════════════════════════════════════════════
# Step 6 — Install built binaries
# ══════════════════════════════════════════════════════════════════════════════
def install_net_helper(project_dir: Path) -> None:
    step("Installing net-helper")
    src = project_dir / "target" / "release" / "net-helper"
    dest = USR_LOCAL_BIN / "net-helper"
    if not src.exists():
        die(f"Binary not found: {src} — run build first")
    sudo("install", "-o", "root", "-g", "root", "-m", "0755", str(src), str(dest))
    sudo("setcap", "cap_net_admin=eip", str(dest))
    ok(f"Installed {dest}  (cap_net_admin=eip)")

def install_server_binary(project_dir: Path) -> None:
    step("Installing server binary")
    src = project_dir / "target" / "release" / "server"
    dest = USR_LOCAL_BIN / "webcode-server"
    if not src.exists():
        die(f"Binary not found: {src} — run build first")
    sudo("install", "-o", "root", "-g", "root", "-m", "0755", str(src), str(dest))
    ok(f"Installed {dest}")

# ══════════════════════════════════════════════════════════════════════════════
# Step 7 — VM SSH key (host → guest)
# ══════════════════════════════════════════════════════════════════════════════
def generate_vm_ssh_key() -> None:
    step("Generating VM SSH key")
    if SSH_KEY.exists():
        skip(str(SSH_KEY))
        return
    with tempfile.TemporaryDirectory() as tmp_str:
        tmp = Path(tmp_str)
        run(["ssh-keygen", "-t", "ed25519", "-f", str(tmp / "id_rsa"), "-N", ""])
        sudo("cp", str(tmp / "id_rsa"), str(SSH_KEY))
        sudo("cp", str(tmp / "id_rsa.pub"), str(SSH_KEY_PUB))
        sudo("chmod", "600", str(SSH_KEY))
    ok(f"Generated {SSH_KEY}")

def generate_vm_host_key() -> None:
    step("Generating VM SSH host key (persistent)")
    if VM_HOST_KEY.exists():
        skip(str(VM_HOST_KEY))
        return
    with tempfile.TemporaryDirectory() as tmp_str:
        tmp = Path(tmp_str)
        tmp_key = tmp / "vm_host_ed25519_key"
        run(["ssh-keygen", "-t", "ed25519", "-f", str(tmp_key), "-N", ""])
        sudo("cp", str(tmp_key), str(VM_HOST_KEY))
        sudo("cp", str(str(tmp_key) + ".pub"), str(VM_HOST_KEY_PUB))
        sudo("chmod", "600", str(VM_HOST_KEY))
    ok(f"Generated {VM_HOST_KEY}")

# ══════════════════════════════════════════════════════════════════════════════
# Step 8 — VM kernel + rootfs
# ══════════════════════════════════════════════════════════════════════════════
def build_rootfs(ci_version: str) -> None:
    step("Building VM kernel + rootfs")

    if KERNEL_DEST.exists():
        skip(str(KERNEL_DEST))
    if ROOTFS_DEST.exists():
        skip(str(ROOTFS_DEST))
    if KERNEL_DEST.exists() and ROOTFS_DEST.exists():
        return

    base_url = f"https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/{ci_version}/{ARCH}"

    with tempfile.TemporaryDirectory() as tmp_str:
        tmp = Path(tmp_str)

        # ── Kernel ──────────────────────────────────────────────────────────
        if not KERNEL_DEST.exists():
            kernel_tmp = tmp / "vmlinux"
            download(f"{base_url}/vmlinux-6.1.155", kernel_tmp)
            sudo("mv", str(kernel_tmp), str(KERNEL_DEST))
            ok(f"Installed kernel → {KERNEL_DEST}")

        # ── Rootfs ──────────────────────────────────────────────────────────
        if not ROOTFS_DEST.exists():
            squashfs_tmp = tmp / "ubuntu.squashfs"
            download(f"{base_url}/ubuntu-24.04.squashfs", squashfs_tmp)

            squashfs_root = tmp / "squashfs-root"
            info("Unpacking squashfs…")
            sudo("unsquashfs", "-d", str(squashfs_root), str(squashfs_tmp))

            # Fix ownership: root for system files, 1000 for ubuntu user
            sudo("chown", "-R", "root:root", str(squashfs_root))
            sudo("chown", "-R", "1000:1000", str(squashfs_root / "home" / "ubuntu"))

            # Authorised SSH key for ubuntu
            ubuntu_ssh = squashfs_root / "home" / "ubuntu" / ".ssh"
            sudo("mkdir", "-p", str(ubuntu_ssh))
            sudo("cp", str(SSH_KEY_PUB), str(ubuntu_ssh / "authorized_keys"))
            sudo("chmod", "700", str(ubuntu_ssh))
            sudo("chmod", "600", str(ubuntu_ssh / "authorized_keys"))
            sudo("chown", "-R", "1000:1000", str(ubuntu_ssh))

            # DNS inside the rootfs
            sudo_write(squashfs_root / "etc" / "resolv.conf", "nameserver 1.1.1.1\n")

            # Inject the persistent SSH host key so it never changes across rebuilds
            etc_ssh = squashfs_root / "etc" / "ssh"
            sudo("mkdir", "-p", str(etc_ssh))
            sudo("cp", str(VM_HOST_KEY), str(etc_ssh / "ssh_host_ed25519_key"))
            sudo("cp", str(VM_HOST_KEY_PUB), str(etc_ssh / "ssh_host_ed25519_key.pub"))
            sudo("chmod", "600", str(etc_ssh / "ssh_host_ed25519_key"))
            ok(f"Injected persistent VM host key → {etc_ssh}")

            # Bind-mount /proc /sys /dev for chroot
            for mount_point in ("proc", "sys", "dev"):
                sudo("mount", "--bind", f"/{mount_point}", str(squashfs_root / mount_point))

            try:
                info("Installing Node.js + Claude Code inside rootfs (chroot)…")
                sudo(
                    "chroot", str(squashfs_root), "bash", "-c",
                    "export DEBIAN_FRONTEND=noninteractive && "
                    "apt-get update -qq && "
                    "apt-get install -y -qq openssh-server nodejs npm && "
                    "npm install -g @anthropic-ai/claude-code",
                )

                # Install settings.py script
                opt_dir = squashfs_root / "opt"
                opt_dir.mkdir(exist_ok=True)
                shutil.copy(str(Path(__file__).parent / "vm" / "settings.py"), str(opt_dir / "settings.py"))

            finally:
                for mount_point in reversed(("proc", "sys", "dev")):
                    sudo("umount", str(squashfs_root / mount_point), check=False)

            # Build 10 GB ext4 image
            ext4_tmp = tmp / "rootfs.ext4"
            info("Creating 10 GB ext4 image (mkfs.ext4)…")
            sudo("bash", "-c", f"truncate -s 10G {ext4_tmp}")
            sudo("mkfs.ext4", "-d", str(squashfs_root), "-F", str(ext4_tmp))
            sudo("mv", str(ext4_tmp), str(ROOTFS_DEST))
            sudo("rm", "-rf", str(squashfs_root))
            ok(f"Installed rootfs → {ROOTFS_DEST}")

# ══════════════════════════════════════════════════════════════════════════════
# Step 9 — config.toml
# ══════════════════════════════════════════════════════════════════════════════
def generate_config(project_dir: Path, database_url: str) -> None:
    step("Writing config.toml")
    config_path = project_dir / "config.toml"
    if config_path.exists():
        skip(str(config_path))
        return

    config_path.write_text(
        f"# Generated by setup.py — edit as needed.\n"
        f"# See config.example.toml for all options.\n"
        f"\n"
        f'kernel_path     = "{KERNEL_DEST}"\n'
        f'rootfs_path     = "{ROOTFS_DEST}"\n'
        f'ssh_key_path    = "{SSH_KEY}"\n'
        f'ssh_user        = "ubuntu"\n'
        f'vm_host_key_path = "{VM_HOST_KEY_PUB}"\n'        f"\n"
        f'user_rootfs_dir = "{USER_ROOTFS_DIR}"\n'
        f'upload_dir      = "/home/ubuntu"\n'
        f"\n"
        f'database_url    = "{database_url}"\n'
        f"port            = 3000\n"
        f"\n"
        f"# Jailer (disabled by default)\n"
        f"# use_jailer         = true\n"
        f'# firecracker_path   = "/usr/local/bin/firecracker"\n'
        f'# jailer_path        = "/usr/local/bin/jailer"\n'
        f"# jailer_uid         = 1000\n"
        f"# jailer_gid         = 1000\n"
        f'# jailer_chroot_base = "{JAILER_CHROOT_BASE}"\n'
        f"\n"
        f"# Cognito OAuth (leave empty to skip authentication)\n"
        f'cognito_client_id     = ""\n'
        f'cognito_client_secret = ""\n'
        f'cognito_domain        = ""\n'
        f'cognito_redirect_uri  = "http://localhost:3000/callback"\n'
        f'cognito_region        = ""\n'
        f'cognito_user_pool_id  = ""\n'
    )
    ok(f"Wrote {config_path}")

# ══════════════════════════════════════════════════════════════════════════════
# Step 10 — systemd service
# ══════════════════════════════════════════════════════════════════════════════
def generate_systemd_service(project_dir: Path) -> None:
    step("Writing systemd service file")
    service_path = project_dir / "webcode-server.service"
    if service_path.exists():
        skip(str(service_path))
        return

    service_path.write_text(
        "[Unit]\n"
        "Description=WebCode Server\n"
        "After=network.target postgresql.service\n"
        "Requires=postgresql.service\n"
        "\n"
        "[Service]\n"
        "Type=simple\n"
        f"WorkingDirectory={project_dir}\n"
        f"ExecStart={USR_LOCAL_BIN / 'webcode-server'}\n"
        "Restart=on-failure\n"
        "RestartSec=5\n"
        "\n"
        "[Install]\n"
        "WantedBy=multi-user.target\n"
    )
    ok(f"Wrote {service_path}")
    info(
        f"To enable:  sudo cp {service_path} /etc/systemd/system/ "
        f"&& sudo systemctl enable --now webcode-server"
    )

# ══════════════════════════════════════════════════════════════════════════════
# Summary
# ══════════════════════════════════════════════════════════════════════════════
def print_summary(project_dir: Path) -> None:
    console.print()
    console.print(
        Panel.fit(
            "[bold green]Setup complete![/bold green]\n\n"
            "[bold]Run the server:[/bold]\n"
            f"  [cyan]cd {project_dir} && ./target/release/server[/cyan]\n\n"
            "[bold]Or via systemd:[/bold]\n"
            f"  [cyan]sudo cp {project_dir}/webcode-server.service /etc/systemd/system/[/cyan]\n"
            f"  [cyan]sudo systemctl enable --now webcode-server[/cyan]\n\n"
            "[bold]Open in browser:[/bold]\n"
            "  [cyan]http://localhost:3000[/cyan]\n\n"
            "[dim]Installed files:\n"
            f"  /usr/local/bin/firecracker      Firecracker microVM\n"
            f"  /usr/local/bin/jailer           Firecracker jailer (setuid root)\n"
            f"  /usr/local/bin/net-helper        TAP / NAT helper (cap_net_admin)\n"
            f"  /usr/local/bin/webcode-server    Web server\n"
            f"  {KERNEL_DEST}          VM kernel\n"
            f"  {ROOTFS_DEST}       VM base rootfs (Ubuntu 24.04 + Claude Code)\n"
            f"  {SSH_KEY}          SSH key for VM access\n"
            f"  {VM_HOST_KEY}  VM SSH host key (private, persistent)\n"
            f"  {VM_HOST_KEY_PUB}  VM SSH host key (public)[/dim]",
            title="[bold]WebCode[/bold]",
            border_style="green",
        )
    )

# ══════════════════════════════════════════════════════════════════════════════
# Main
# ══════════════════════════════════════════════════════════════════════════════
def main() -> None:
    parser = argparse.ArgumentParser(
        description="WebCode — full setup script",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--firecracker-version", default=FC_VERSION, metavar="VER",
        help=f"Firecracker release version to download (default: {FC_VERSION})",
    )
    parser.add_argument(
        "--ci-version", default=FC_CI_VERSION, metavar="VER",
        help=f"Firecracker CI image version for kernel/rootfs (default: {FC_CI_VERSION})",
    )
    parser.add_argument(
        "--database-url", default="postgres://localhost/webcode", metavar="URL",
        help="PostgreSQL connection URL (default: postgres://localhost/webcode)",
    )
    parser.add_argument("--skip-firecracker", action="store_true",
                        help="Skip downloading Firecracker + jailer binaries")
    parser.add_argument("--skip-rootfs", action="store_true",
                        help="Skip building the VM kernel + rootfs (the slow step)")
    parser.add_argument("--skip-build", action="store_true",
                        help="Skip cargo build --release")
    parser.add_argument("--skip-db", action="store_true",
                        help="Skip PostgreSQL database creation")
    args = parser.parse_args()

    project_dir = Path(__file__).parent.resolve()

    console.print(
        Panel.fit(
            f"[bold]WebCode Setup[/bold]\n"
            f"Project dir : {project_dir}\n"
            f"Firecracker : v{args.firecracker_version}   "
            f"CI images: {args.ci_version}   "
            f"DB: {args.database_url}",
            border_style="cyan",
        )
    )

    check_prerequisites()
    setup_directories()

    if not args.skip_firecracker:
        install_firecracker(args.firecracker_version)

    if not args.skip_db:
        setup_database(args.database_url)

    if not args.skip_build:
        build_project(project_dir, args.database_url)
        install_net_helper(project_dir)
        install_server_binary(project_dir)

    generate_vm_ssh_key()
    generate_vm_host_key()

    if not args.skip_rootfs:
        build_rootfs(args.ci_version)

    generate_config(project_dir, args.database_url)
    generate_systemd_service(project_dir)
    print_summary(project_dir)


if __name__ == "__main__":
    main()
