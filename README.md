# vm-terminal

Browser-based terminal that boots a Firecracker microVM per WebSocket connection. Each VM gets its own network interface and SSH session; the browser talks to the VM via xterm.js → WebSocket → SSH.

## Architecture

```
Browser (xterm.js)
    ↕ WebSocket (binary = terminal data, text JSON = resize)
server (axum)
    ↕ SSH (russh) over TAP network 172.16.{n}.1/30
Firecracker VM (sshd)
```

Each WebSocket connection:
1. Generates a Firecracker VM with a dedicated TAP interface
2. Injects kernel `ip=` arg so `eth0` is configured before userspace
3. Opens an SSH session to the VM, requests a PTY, and relays data
4. Cleans up the VM and TAP on disconnect

## Prerequisites

- Rust toolchain
- `ip` and `iptables` on `$PATH`
- User in the `kvm` group (`sudo usermod -aG kvm $USER`)
- A kernel image and a per-user root filesystem (see [Building the root filesystem](#building-the-root-filesystem))

## Build

```bash
cargo build --release
```

## Install net-helper

`net-helper` handles privileged network operations (TAP creation, NAT setup) so the server runs without root.

```bash
sudo install -o root -g root -m 0755 target/release/net-helper /usr/local/bin/net-helper
sudo setcap cap_net_admin=eip /usr/local/bin/net-helper
```

## Install Firecracker and Jailer

Download the release archive from the [Firecracker releases page](https://github.com/firecracker-microvm/firecracker/releases) and install both binaries:

```bash
tar -xzf firecracker-v1.x.x-x86_64.tgz
sudo install -o root -g root -m 0755 firecracker-v1.x.x-x86_64 /usr/local/bin/firecracker
sudo install -o root -g root -m 0755 jailer-v1.x.x-x86_64      /usr/local/bin/jailer
sudo chmod u+s /usr/local/bin/jailer
```

The jailer binary must be setuid root so it can chroot and drop privileges without the server running as root.

## Jailer setup

The jailer chroots each Firecracker process into a per-VM directory tree and drops it to a dedicated uid/gid, providing process isolation.

### Chroot directory layout

With the default `jailer_chroot_base = /srv/jailer`, each VM gets the following tree on the host:

```
/srv/jailer/
└── firecracker/
    └── {vm-uuid}/
        └── root/                      ← chroot root (owned by jailer_uid:jailer_gid, mode 0700)
            ├── vmlinux                ← hard-link of the kernel image
            ├── rootfs.ext4            ← per-user rootfs copy
            └── run/
                └── firecracker.socket ← Firecracker API socket
```

Inside the jail, the API socket appears at `/run/firecracker.socket`. From the host, the full path is `/srv/jailer/firecracker/{vm-uuid}/root/run/firecracker.socket`.

The server creates the `root/` subtree before spawning the jailer. The jailer then `chown`s `root/` to `jailer_uid:jailer_gid` (mode `0700`) and `chroot`s into it. After the VM exits, the server deletes the entire `{vm-uuid}/` directory.

### User and permissions

The jailer `chown`s the chroot directory to `jailer_uid:jailer_gid`. For the server to be able to delete the chroot tree after the VM exits, **`jailer_uid` and `jailer_gid` must match the uid/gid of the user running the server**.

1. Create a dedicated system user for the server and its VMs:

```bash
sudo useradd -r -m -d /var/lib/webcode -s /sbin/nologin webcode
sudo usermod -aG kvm webcode
```

2. Create the chroot base directory, owned by that user:

```bash
sudo mkdir -p /srv/jailer
sudo chown webcode:webcode /srv/jailer
```

3. Set `jailer_uid` and `jailer_gid` to the new user's IDs:

```bash
id webcode
# uid=999(webcode) gid=999(webcode) ...
```

```toml
jailer_uid = 999  # id -u webcode
jailer_gid = 999  # id -g webcode
```

4. Run the server as `webcode` (e.g. via systemd `User=webcode`).

## Building the root filesystem

Use the latest Firecracker minor version (e.g. `v1.14`) and pick the highest kernel version from the S3 listing at `http://spec.ccfc.min.s3.amazonaws.com/?prefix=firecracker-ci/v1.14/x86_64/vmlinux-&list-type=2`.

### Automated (recommended)

`scripts/build_rootfs.py` automates the full build. It requires root, `uv`, `unsquashfs`, and `mkfs.ext4`:

```bash
sudo $(which uv) run scripts/build_rootfs.py
```

Pass `--workdir /path/to/dir` to use a specific working directory (defaults to a temporary directory). The script installs the finished artifacts directly into `/var/lib/fc/`.

> **Note:** `sudo $(which uv)` is needed because `sudo` resets PATH and won't find `uv` otherwise.

### Manual steps

```bash
# Download
wget "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/vmlinux-6.1.155"
wget -O ubuntu-24.04.squashfs.upstream "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/ubuntu-24.04.squashfs"

# Unpack and set ownership
unsquashfs ubuntu-24.04.squashfs.upstream
sudo chown -R root:root squashfs-root
sudo chown -R 1000:1000 squashfs-root/home/ubuntu

# SSH key
ssh-keygen -t ed25519 -f id_ed25519 -N ""
mkdir -p squashfs-root/home/ubuntu/.ssh
cp id_ed25519.pub squashfs-root/home/ubuntu/.ssh/authorized_keys
sudo chmod 700 squashfs-root/home/ubuntu/.ssh
sudo chmod 600 squashfs-root/home/ubuntu/.ssh/authorized_keys
echo "nameserver 1.1.1.1" | sudo tee squashfs-root/etc/resolv.conf > /dev/null

# Install uv (system-wide) and Claude Code CLI (ubuntu user)
sudo chmod 1777 squashfs-root/tmp
sudo mkdir -p squashfs-root/var/cache/apt/archives/partial
sudo mkdir -p squashfs-root/var/log/apt
sudo mount --bind /proc squashfs-root/proc
sudo mount --bind /sys  squashfs-root/sys
sudo mount --bind /dev  squashfs-root/dev
sudo chroot squashfs-root bash -c "
  apt-get update -qq &&
  apt-get install -y -qq curl &&
  curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh
"
sudo chroot squashfs-root su - ubuntu -c "bash -c 'curl -fsSL https://claude.ai/install.sh | bash && echo '\''export PATH=\"\$HOME/.local/bin:\$PATH\"'\'' >> ~/.bashrc'"

# Place the agent and pre-warm the uv dependency cache as the ubuntu user
sudo cp rootfs/agent.py squashfs-root/opt/agent.py
sudo chroot squashfs-root su - ubuntu -c "echo | bash -lc 'uv run /opt/agent.py'" || true

sudo umount squashfs-root/dev
sudo umount squashfs-root/sys
sudo umount squashfs-root/proc

# Claude Code settings
sudo mkdir -p squashfs-root/home/ubuntu/.claude
sudo tee squashfs-root/home/ubuntu/.claude/settings.json > /dev/null << 'EOF'
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "env": {
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "us.anthropic.claude-opus-4-6-v1",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "us.anthropic.claude-sonnet-4-6",
    "CLAUDE_CODE_USE_BEDROCK": "1"
  }
}
EOF
sudo chown -R 1000:1000 squashfs-root/home/ubuntu/.claude

# Build ext4 image
truncate -s 10G ubuntu-24.04.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-24.04.ext4
sudo rm -rf squashfs-root
mv id_ed25519 ubuntu-24.04.id_ed25519

# Install to /var/lib/fc
sudo mkdir -p /var/lib/fc
sudo mv vmlinux-6.1.155 /var/lib/fc/vmlinux
sudo mv ubuntu-24.04.ext4 /var/lib/fc/ubuntu-24.04.ext4
sudo mv id_ed25519 /var/lib/fc/ubuntu-24.04.id_ed25519
```

The rootfs contains:
- **`claude` CLI** (installed per-user via `https://claude.ai/install.sh`) — the Claude Code binary invoked by the agent
- **`uv`** (system-wide at `/usr/local/bin/uv`) — runs `agent.py` and manages its Python dependencies
- **`/opt/agent.py`** — reads JSON queries from stdin, streams results back via stdout using the `claude-agent-sdk` Python package
- **Claude Code settings** — configured for AWS Bedrock; credentials are inherited from the host instance profile at runtime

## Run

Configuration is loaded from `config.toml` (optional) and environment variables. A minimal `config.toml`:

```toml
kernel_path   = "/var/lib/fc/vmlinux"
rootfs_path   = "/var/lib/fc/ubuntu-24.04.ext4"
ssh_key_path  = "/var/lib/fc/ubuntu-24.04.id_ed25519"
ssh_user      = "ubuntu"
jailer_uid    = 999  # id -u webcode
jailer_gid    = 999  # id -g webcode

cognito_client_id     = "..."
cognito_client_secret = "..."
cognito_region        = "us-east-1"
cognito_user_pool_id  = "us-east-1_xxxxxxxx"
cognito_domain        = "your-domain.auth.us-east-1.amazoncognito.com"
cognito_redirect_uri  = "https://yourhost/callback"
```

```bash
./target/release/server
```

## User system

Each visitor must log in via Cognito before creating or connecting to VMs. VMs are scoped per user — each user only sees and can interact with their own VMs. Sessions are in-memory; restarting the server logs everyone out and destroys all running VMs.

**Authentication flow:**
1. Visit any page → redirected to `/login`
2. `/login` → redirected to Cognito hosted UI
3. Cognito authenticates → redirects to `/callback`
4. Session established, user lands on `/vms`

## UI

- `/vms` — VM list (server-rendered, no JavaScript)
- `/terminal/{id}` — in-browser terminal (xterm.js + WebSocket only)

Open http://localhost:3000 — each page load boots a fresh VM and opens an SSH terminal in the browser.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `KERNEL_PATH` | `/var/lib/fc/vmlinux` | Firecracker kernel image |
| `ROOTFS_PATH` | `/var/lib/fc/rootfs.ext4` | Root filesystem image |
| `SSH_KEY_PATH` | `/var/lib/fc/id_ed25519` | SSH private key matching the public key baked into the rootfs |
| `SSH_USER` | `root` | SSH login user inside the VM |
| `VM_HOST_KEY_PATH` | `/var/lib/fc/vm_host_key.pub` | Known-host public key for the VM's sshd (prevents MITM on the internal TAP network) |
| `PORT` | `3000` | HTTP listen port |
| `NET_HELPER_PATH` | `/usr/local/bin/net-helper` | Path to the net-helper binary |
| `JAILER_PATH` | `/usr/local/bin/jailer` | Path to the jailer binary |
| `FIRECRACKER_PATH` | `/usr/local/bin/firecracker` | Path to the firecracker binary |
| `JAILER_UID` | `0` | uid the jailer drops Firecracker to; must match the server process uid |
| `JAILER_GID` | `0` | gid the jailer drops Firecracker to; must match the server process gid |
| `JAILER_CHROOT_BASE` | `/srv/jailer` | Base directory for per-VM chroot trees |
| `COGNITO_CLIENT_ID` | — | Cognito app client ID |
| `COGNITO_CLIENT_SECRET` | — | Cognito app client secret |
| `COGNITO_REGION` | — | AWS region of the user pool |
| `COGNITO_USER_POOL_ID` | — | Cognito user pool ID |
| `COGNITO_DOMAIN` | — | Cognito hosted UI domain |
| `COGNITO_REDIRECT_URI` | `http://localhost:3000/callback` | OAuth2 redirect URI |

## Networking

The server sets up a `/30` point-to-point network per VM:

| Address | Role |
|---|---|
| `172.16.{n}.1` | Host-side TAP interface |
| `172.16.{n}.2` | VM `eth0` (configured via kernel `ip=` cmdline) |

NAT masquerading is configured automatically on startup via `net-helper setup-nat`.

The VM can reach the internet via the host's default route. The AWS IMDS endpoint (`169.254.169.254`) resolves to the MMDS served by Firecracker.
