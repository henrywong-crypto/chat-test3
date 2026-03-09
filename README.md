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
- `firecracker` binary on `$PATH`
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

## Jailer setup (optional)

The jailer chroots each Firecracker process into its own directory and drops it to a dedicated uid/gid, providing process isolation. To enable it:

1. Create a dedicated system user for Firecracker:

```bash
sudo useradd -r -s /sbin/nologin firecracker
```

2. Create and grant access to the chroot base directory:

```bash
sudo mkdir -p /srv/jailer
sudo chown ubuntu:ubuntu /srv/jailer
```

3. Enable in `config.toml`:

```toml
use_jailer         = true
jailer_uid         = 1001  # id -u firecracker
jailer_gid         = 1001  # id -g firecracker
```

## Building the root filesystem

Use the latest Firecracker minor version (e.g. `v1.14`) and pick the highest kernel version from the S3 listing at `http://spec.ccfc.min.s3.amazonaws.com/?prefix=firecracker-ci/v1.14/x86_64/vmlinux-&list-type=2`.

```bash
# Download
wget "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/vmlinux-6.1.155"
wget -O ubuntu-24.04.squashfs.upstream "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/ubuntu-24.04.squashfs"

# Unpack and set ownership
unsquashfs ubuntu-24.04.squashfs.upstream
sudo chown -R root:root squashfs-root
sudo chown -R 1000:1000 squashfs-root/home/ubuntu

# SSH key
ssh-keygen -f id_rsa -N ""
mkdir -p squashfs-root/home/ubuntu/.ssh
cp id_rsa.pub squashfs-root/home/ubuntu/.ssh/authorized_keys
sudo chmod 700 squashfs-root/home/ubuntu/.ssh
sudo chmod 600 squashfs-root/home/ubuntu/.ssh/authorized_keys
echo "nameserver 1.1.1.1" | sudo tee squashfs-root/etc/resolv.conf > /dev/null

# Install Node.js and Claude Code
sudo chmod 1777 squashfs-root/tmp
sudo mkdir -p squashfs-root/var/cache/apt/archives/partial
sudo mkdir -p squashfs-root/var/log/apt
sudo mount --bind /proc squashfs-root/proc
sudo mount --bind /sys  squashfs-root/sys
sudo mount --bind /dev  squashfs-root/dev
sudo chroot squashfs-root bash -c "
  apt-get update -qq &&
  apt-get install -y -qq nodejs npm &&
  npm install -g @anthropic-ai/claude-code
"
sudo umount squashfs-root/dev
sudo umount squashfs-root/sys
sudo umount squashfs-root/proc

# Claude Code settings
mkdir -p squashfs-root/home/ubuntu/.claude
cat > squashfs-root/home/ubuntu/.claude/settings.json << 'EOF'
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

# Build ext4 image
truncate -s 10G ubuntu-24.04.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-24.04.ext4
sudo rm -rf squashfs-root
mv id_rsa ubuntu-24.04.id_rsa

# Install to /var/lib/fc
sudo mkdir -p /var/lib/fc
sudo mv vmlinux-6.1.155 /var/lib/fc/vmlinux
sudo mv ubuntu-24.04.ext4 /var/lib/fc/ubuntu-24.04.ext4
sudo mv ubuntu-24.04.id_rsa /var/lib/fc/ubuntu-24.04.id_rsa
```

## Run

Configure via environment variables:

```bash
KERNEL_PATH=~/vmlinux-6.1.155 \
ROOTFS_PATH=~/ubuntu-24.04.ext4 \
SSH_KEY_PATH=~/ubuntu-24.04.id_rsa \
SSH_USER=ubuntu \
./target/release/server
```

For Cognito login, also set:

```bash
COGNITO_CLIENT_ID=your_client_id \
COGNITO_CLIENT_SECRET=your_client_secret \
COGNITO_REGION=us-east-1 \
COGNITO_USER_POOL_ID=us-east-1_xxxxxxxx \
COGNITO_DOMAIN=your-domain.auth.us-east-1.amazoncognito.com \
COGNITO_REDIRECT_URI=http://localhost:3000/callback \
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
| `SSH_KEY_PATH` | `/var/lib/fc/id_rsa` | SSH private key matching the public key baked into the rootfs |
| `SSH_USER` | `root` | SSH login user inside the VM |
| `SOCKET_DIR` | `/tmp` | Directory for Firecracker API sockets |
| `PORT` | `3000` | HTTP listen port |
| `NET_HELPER_PATH` | `/usr/local/bin/net-helper` | Path to the net-helper binary |
| `AWS_ROLE_NAME` | `vm-role` | IAM role name forwarded to the VM via MMDS |
| `USE_JAILER` | `false` | Enable jailer process isolation |
| `JAILER_PATH` | `/usr/local/bin/jailer` | Path to the jailer binary |
| `FIRECRACKER_PATH` | `/usr/local/bin/firecracker` | Path to the firecracker binary |
| `JAILER_UID` | `0` | uid to drop privileges to inside the jail |
| `JAILER_GID` | `0` | gid to drop privileges to inside the jail |
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
