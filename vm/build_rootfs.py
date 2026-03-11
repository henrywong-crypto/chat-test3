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
import json
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

S3_BASE = "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci"
S3_LIST = "https://s3.amazonaws.com/spec.ccfc.min"
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
curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh
"""

# Runs as the ubuntu user inside the chroot: installs Claude Code CLI.
UBUNTU_INSTALL_SCRIPT = """\
set -e
curl -fsSL https://claude.ai/install.sh | bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
"""


def run(cmd: list, **kwargs) -> None:
    print(f"+ {' '.join(str(c) for c in cmd)}")
    subprocess.run(cmd, check=True, **kwargs)


def fetch_arch() -> str:
    return subprocess.check_output(["uname", "-m"], text=True).strip()


def fetch_latest_fc_version() -> str:
    url = "https://api.github.com/repos/firecracker-microvm/firecracker/releases/latest"
    req = urllib.request.Request(url, headers={"User-Agent": "build-rootfs"})
    with urllib.request.urlopen(req) as resp:
        tag = json.loads(resp.read())["tag_name"]  # e.g. "v1.14.0"
    parts = tag.lstrip("v").split(".")
    return f"v{parts[0]}.{parts[1]}"


def fetch_s3_keys(fc_version: str, arch: str, prefix: str) -> list[str]:
    url = f"{S3_LIST}/?prefix=firecracker-ci/{fc_version}/{arch}/{prefix}&list-type=2"
    with urllib.request.urlopen(url) as resp:
        root = ET.fromstring(resp.read())
    ns = {"s3": "http://s3.amazonaws.com/doc/2006-03-01/"}
    return [el.text for el in root.findall(".//s3:Key", ns) if el.text]


def fetch_latest_kernel_key(fc_version: str, arch: str) -> str:
    keys = fetch_s3_keys(fc_version, arch, "vmlinux-")
    # Keep only plain versioned kernels (e.g. vmlinux-6.1.155), not vmlinux-acpi-* etc.
    versioned = [k for k in keys if Path(k).name.count("-") == 1
                 and all(p.isdigit() for p in Path(k).name.split("-", 1)[1].split("."))]
    if not versioned:
        sys.exit(f"error: no kernel images found for Firecracker {fc_version}/{arch}")
    versioned.sort(key=lambda k: tuple(int(x) for x in k.rsplit("-", 1)[-1].split(".")))
    return versioned[-1]


def fetch_latest_ubuntu_key(fc_version: str, arch: str) -> str:
    keys = [k for k in fetch_s3_keys(fc_version, arch, "ubuntu-") if k.endswith(".squashfs")]
    if not keys:
        sys.exit(f"error: no Ubuntu squashfs found for Firecracker {fc_version}/{arch}")
    return sorted(keys)[-1]  # e.g. "firecracker-ci/v1.14/x86_64/ubuntu-24.04.squashfs"


def download_artifacts(
    workdir: Path, fc_version: str, arch: str, kernel_key: str, ubuntu_key: str
) -> tuple[Path, Path]:
    s3_base = f"{S3_BASE}/{fc_version}/{arch}"
    kernel_name = Path(kernel_key).name        # e.g. "vmlinux-6.1.155"
    ubuntu_name = Path(ubuntu_key).stem        # e.g. "ubuntu-24.04"
    kernel = workdir / kernel_name
    squashfs = workdir / f"{ubuntu_name}.squashfs.upstream"
    if not kernel.exists():
        run(["wget", "-O", str(kernel), f"{s3_base}/{kernel_name}"])
    if not squashfs.exists():
        run(["wget", "-O", str(squashfs), f"{s3_base}/{ubuntu_name}.squashfs"])
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
    # bash -l sources ~/.profile → ~/.bashrc so the claude binary is on PATH.
    run(
        ["chroot", str(rootfs), "su", "-", "ubuntu", "-c",
         "echo | bash -lc '/usr/local/bin/uv run /opt/agent.py'"],
    )


def write_claude_settings(rootfs: Path) -> None:
    claude_dir = rootfs / "home/ubuntu/.claude"
    claude_dir.mkdir(parents=True, exist_ok=True)
    (claude_dir / "settings.json").write_text(CLAUDE_SETTINGS)
    run(["chown", "-R", "1000:1000", str(claude_dir)])


def build_ext4(workdir: Path, rootfs: Path, ubuntu_name: str) -> Path:
    ext4 = workdir / f"{ubuntu_name}.ext4"
    if ext4.exists():
        ext4.unlink()
    run(["truncate", "-s", "10G", str(ext4)])
    run(["mkfs.ext4", "-d", str(rootfs), "-F", str(ext4)])
    run(["rm", "-rf", str(rootfs)])
    return ext4


def install_artifacts(
    kernel: Path, ext4: Path, ssh_key: Path, ubuntu_name: str
) -> tuple[Path, Path, Path]:
    INSTALL_DIR.mkdir(parents=True, exist_ok=True)
    kernel_dest = INSTALL_DIR / kernel.name          # e.g. vmlinux-6.1.155
    ext4_dest = INSTALL_DIR / ext4.name              # e.g. ubuntu-24.04.ext4
    ssh_key_dest = INSTALL_DIR / f"{ubuntu_name}.id_rsa"
    shutil.move(str(kernel), str(kernel_dest))
    shutil.move(str(ext4), str(ext4_dest))
    shutil.move(str(ssh_key), str(ssh_key_dest))
    # Make files readable by the server user (non-root).
    # protected_hardlinks blocks hard-linking root-owned files that aren't world-readable.
    kernel_dest.chmod(0o644)
    ext4_dest.chmod(0o644)
    # The SSH key is the server's identity key for connecting to VMs; it must be
    # readable by the server process user.
    ssh_key_dest.chmod(0o644)
    return kernel_dest, ext4_dest, ssh_key_dest


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

    arch = fetch_arch()
    print(f"Architecture: {arch}")
    print("Fetching latest versions...")
    fc_version = fetch_latest_fc_version()
    kernel_key = fetch_latest_kernel_key(fc_version, arch)
    ubuntu_key = fetch_latest_ubuntu_key(fc_version, arch)
    ubuntu_name = Path(ubuntu_key).stem  # e.g. "ubuntu-24.04"
    print(f"Firecracker {fc_version}, kernel {Path(kernel_key).name}, rootfs {ubuntu_name}")

    kernel, squashfs = download_artifacts(workdir, fc_version, arch, kernel_key, ubuntu_key)
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
    ext4 = build_ext4(workdir, rootfs, ubuntu_name)
    kernel_dest, ext4_dest, ssh_key_dest = install_artifacts(kernel, ext4, ssh_key, ubuntu_name)

    print(f"\nDone. Artifacts installed to {INSTALL_DIR}/:")
    print(f"  {kernel_dest}")
    print(f"  {ext4_dest}")
    print(f"  {ssh_key_dest}")


if __name__ == "__main__":
    main()
