#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Build the Firecracker rootfs for WebCode.

Must be run as root. Because sudo resets PATH, pass uv's full path:

    sudo $(which uv) run vm/build_rootfs.py

Use --workdir to keep artifacts between runs (avoids re-downloading):

    sudo $(which uv) run vm/build_rootfs.py --workdir /tmp/fc-build
"""

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

KERNEL_VERSION = "6.1.155"
FC_VERSION = "v1.14"
S3_BASE = f"https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/{FC_VERSION}/x86_64"
KERNEL_URL = f"{S3_BASE}/vmlinux-{KERNEL_VERSION}"
SQUASHFS_URL = f"{S3_BASE}/ubuntu-24.04.squashfs"
INSTALL_DIR = Path("/var/lib/fc")
AGENT_PY = Path(__file__).parent / "agent.py"

CLAUDE_SETTINGS = """\
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "env": {
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "us.anthropic.claude-opus-4-6-v1",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "us.anthropic.claude-sonnet-4-6",
    "CLAUDE_CODE_USE_BEDROCK": "1"
  }
}
"""

# Runs as root inside the chroot: installs system packages and uv.
CHROOT_INSTALL_SCRIPT = """\
set -e
apt-get update -qq
apt-get install -y -qq curl
curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local sh
"""

# Runs as the ubuntu user inside the chroot: installs Claude Code CLI.
UBUNTU_INSTALL_SCRIPT = """\
set -e
curl -fsSL https://claude.ai/install.sh | bash
"""


def run(cmd: list, **kwargs) -> None:
    print(f"+ {' '.join(str(c) for c in cmd)}")
    subprocess.run(cmd, check=True, **kwargs)


def download_artifacts(workdir: Path) -> tuple[Path, Path]:
    kernel = workdir / f"vmlinux-{KERNEL_VERSION}"
    squashfs = workdir / "ubuntu-24.04.squashfs.upstream"
    if not kernel.exists():
        run(["wget", "-O", str(kernel), KERNEL_URL])
    if not squashfs.exists():
        run(["wget", "-O", str(squashfs), SQUASHFS_URL])
    return kernel, squashfs


def unpack_squashfs(workdir: Path, squashfs: Path) -> Path:
    rootfs = workdir / "squashfs-root"
    if rootfs.exists():
        run(["rm", "-rf", str(rootfs)])
    run(["unsquashfs", "-d", str(rootfs), str(squashfs)])
    run(["chown", "-R", "root:root", str(rootfs)])
    run(["chown", "-R", "1000:1000", str(rootfs / "home/ubuntu")])
    return rootfs


def setup_ssh_key(workdir: Path, rootfs: Path) -> Path:
    ssh_key = workdir / "id_rsa"
    if ssh_key.exists():
        ssh_key.unlink()
        ssh_key.with_suffix(".pub").unlink(missing_ok=True)
    run(["ssh-keygen", "-t", "rsa", "-f", str(ssh_key), "-N", ""])
    ssh_dir = rootfs / "home/ubuntu/.ssh"
    ssh_dir.mkdir(mode=0o700, exist_ok=True)
    shutil.copy(ssh_key.with_suffix(".pub"), ssh_dir / "authorized_keys")
    run(["chmod", "700", str(ssh_dir)])
    run(["chmod", "600", str(ssh_dir / "authorized_keys")])
    run(["chown", "-R", "1000:1000", str(ssh_dir)])
    return ssh_key


def prepare_rootfs(rootfs: Path) -> None:
    (rootfs / "etc/resolv.conf").write_text("nameserver 1.1.1.1\n")
    run(["chmod", "1777", str(rootfs / "tmp")])
    (rootfs / "var/cache/apt/archives/partial").mkdir(parents=True, exist_ok=True)
    (rootfs / "var/log/apt").mkdir(parents=True, exist_ok=True)


def mount_binds(rootfs: Path) -> list[Path]:
    mounts = [rootfs / "proc", rootfs / "sys", rootfs / "dev"]
    for mount_path in mounts:
        run(["mount", "--bind", f"/{mount_path.name}", str(mount_path)])
    return mounts


def unmount_binds(mounts: list[Path]) -> None:
    for mount_path in reversed(mounts):
        subprocess.run(["umount", str(mount_path)], check=False)


def install_base_packages(rootfs: Path) -> None:
    run(["chroot", str(rootfs), "bash", "-c", CHROOT_INSTALL_SCRIPT])


def install_claude_user(rootfs: Path) -> None:
    # Write to a temp file to avoid shell-quoting complexity.
    script_path = rootfs / "tmp/install-claude.sh"
    script_path.write_text(UBUNTU_INSTALL_SCRIPT)
    run(["chroot", str(rootfs), "su", "-", "ubuntu", "-c", "bash /tmp/install-claude.sh"])
    script_path.unlink()


def install_agent(rootfs: Path) -> None:
    (rootfs / "opt").mkdir(exist_ok=True)
    shutil.copy(str(AGENT_PY), str(rootfs / "opt/agent.py"))
    # Pre-warm the uv cache as the ubuntu user so the first VM startup is instant.
    # Sending an immediate EOF to stdin causes the agent to exit cleanly after
    # downloading and caching the claude-code-sdk dependency.
    # bash -l sources ~/.profile → ~/.bashrc so nvm's node/claude are on PATH.
    run(
        ["chroot", str(rootfs), "su", "-", "ubuntu", "-c",
         "echo | bash -lc '/usr/local/bin/uv run /opt/agent.py'"],
    )


def write_claude_settings(rootfs: Path) -> None:
    claude_dir = rootfs / "home/ubuntu/.claude"
    claude_dir.mkdir(parents=True, exist_ok=True)
    (claude_dir / "settings.json").write_text(CLAUDE_SETTINGS)
    run(["chown", "-R", "1000:1000", str(claude_dir)])


def build_ext4(workdir: Path, rootfs: Path) -> Path:
    ext4 = workdir / "ubuntu-24.04.ext4"
    if ext4.exists():
        ext4.unlink()
    run(["truncate", "-s", "10G", str(ext4)])
    run(["mkfs.ext4", "-d", str(rootfs), "-F", str(ext4)])
    run(["rm", "-rf", str(rootfs)])
    return ext4


def install_artifacts(kernel: Path, ext4: Path, ssh_key: Path) -> None:
    INSTALL_DIR.mkdir(parents=True, exist_ok=True)
    shutil.move(str(kernel), str(INSTALL_DIR / "vmlinux"))
    shutil.move(str(ext4), str(INSTALL_DIR / "ubuntu-24.04.ext4"))
    shutil.move(str(ssh_key), str(INSTALL_DIR / "ubuntu-24.04.id_rsa"))


def main() -> None:
    if sys.platform != "linux":
        sys.exit("error: rootfs build requires Linux (for chroot and bind mounts)")

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workdir",
        type=Path,
        default=None,
        help="Directory for intermediate files (default: a fresh temp dir)",
    )
    args = parser.parse_args()

    workdir = args.workdir or Path(tempfile.mkdtemp(prefix="fc-build-"))
    workdir.mkdir(parents=True, exist_ok=True)
    print(f"Working directory: {workdir}")

    kernel, squashfs = download_artifacts(workdir)
    rootfs = unpack_squashfs(workdir, squashfs)
    ssh_key = setup_ssh_key(workdir, rootfs)
    prepare_rootfs(rootfs)

    mounts = mount_binds(rootfs)
    try:
        install_base_packages(rootfs)
        install_claude_user(rootfs)
        install_agent(rootfs)
    finally:
        unmount_binds(mounts)

    write_claude_settings(rootfs)
    ext4 = build_ext4(workdir, rootfs)
    install_artifacts(kernel, ext4, ssh_key)

    print("\nDone. Artifacts installed to /var/lib/fc/:")
    print(f"  {INSTALL_DIR}/vmlinux")
    print(f"  {INSTALL_DIR}/ubuntu-24.04.ext4")
    print(f"  {INSTALL_DIR}/ubuntu-24.04.id_rsa")


if __name__ == "__main__":
    main()
