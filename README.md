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

## Building the root filesystem

### 1. Find the latest versions

**Firecracker version** — check https://github.com/firecracker-microvm/firecracker/releases/latest and take the minor version, e.g. `v1.14.2` → `v1.14`.

**Kernel and rootfs keys** — list available files for your chosen version and arch (`x86_64` or `aarch64`):

```
http://spec.ccfc.min.s3.amazonaws.com/?prefix=firecracker-ci/v1.14/x86_64/vmlinux-&list-type=2
http://spec.ccfc.min.s3.amazonaws.com/?prefix=firecracker-ci/v1.14/x86_64/ubuntu-24.04&list-type=2
```

Pick the highest kernel version from the `<Key>` tags, e.g. `firecracker-ci/v1.14/x86_64/vmlinux-6.1.155`.

### 2. Download kernel and rootfs

```bash
wget "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/vmlinux-6.1.155"
wget -O ubuntu-24.04.squashfs.upstream "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/ubuntu-24.04.squashfs"
```

### 3. Build the ext4 rootfs

```bash
# Unpack
sudo unsquashfs ubuntu-24.04.squashfs.upstream

# Generate SSH keypair
ssh-keygen -f id_rsa -N ""

# Bake in the public key and DNS
mkdir -p squashfs-root/root/.ssh
cp id_rsa.pub squashfs-root/root/.ssh/authorized_keys
echo "nameserver 8.8.8.8" > squashfs-root/etc/resolv.conf

# Build ext4 image
sudo chown -R root:root squashfs-root
truncate -s 1G ubuntu-24.04.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-24.04.ext4

# Cleanup
mv id_rsa ubuntu-24.04.id_rsa
sudo rm -rf squashfs-root
```

#### Login as `ubuntu` instead of `root`

Add the SSH key to the `ubuntu` user's home directory instead of (or in addition to) `root`:

```bash
sudo unsquashfs ubuntu-24.04.squashfs.upstream

ssh-keygen -f id_rsa -N ""

mkdir -p squashfs-root/home/ubuntu/.ssh
cp id_rsa.pub squashfs-root/home/ubuntu/.ssh/authorized_keys
sudo chown -R 1000:1000 squashfs-root/home/ubuntu/.ssh
chmod 700 squashfs-root/home/ubuntu/.ssh
chmod 600 squashfs-root/home/ubuntu/.ssh/authorized_keys
echo "nameserver 8.8.8.8" > squashfs-root/etc/resolv.conf

sudo chown -R root:root squashfs-root
truncate -s 1G ubuntu-24.04.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-24.04.ext4

mv id_rsa ubuntu-24.04.id_rsa
sudo rm -rf squashfs-root
```

Then set `SSH_USER=ubuntu` when running the server (see [Run](#run)).

If you already have a built rootfs you don't want to rebuild, inject the key in-place:

```bash
sudo mkdir -p /mnt/rootfs
sudo mount /var/lib/fc/ubuntu-24.04.ext4 /mnt/rootfs

sudo mkdir -p /mnt/rootfs/home/ubuntu/.ssh
sudo cp /var/lib/fc/ubuntu-24.04.id_rsa.pub /mnt/rootfs/home/ubuntu/.ssh/authorized_keys
sudo chown -R 1000:1000 /mnt/rootfs/home/ubuntu/.ssh
sudo chmod 700 /mnt/rootfs/home/ubuntu/.ssh
sudo chmod 600 /mnt/rootfs/home/ubuntu/.ssh/authorized_keys

sudo umount /mnt/rootfs
```

### 4. Install to /var/lib/fc

```bash
sudo mkdir -p /var/lib/fc
sudo mv vmlinux-6.1.155 /var/lib/fc/vmlinux
sudo mv ubuntu-24.04.ext4 /var/lib/fc/ubuntu-24.04.ext4
sudo mv ubuntu-24.04.id_rsa /var/lib/fc/ubuntu-24.04.id_rsa
```

## Run

Create a `config.toml` in the working directory (all fields are optional — defaults are shown):

```toml
kernel_path  = "/var/lib/fc/vmlinux"
rootfs_path  = "/var/lib/fc/ubuntu-24.04.ext4"
ssh_key_path = "/var/lib/fc/ubuntu-24.04.id_rsa"
ssh_user     = "ubuntu"   # default is "root"
socket_dir   = "/tmp"
port         = "3000"

# AWS Cognito — required for the user login system
cognito_client_id     = "your_client_id"
cognito_client_secret = "your_client_secret"
cognito_region        = "us-east-1"
cognito_user_pool_id  = "us-east-1_xxxxxxxx"
cognito_domain        = "your-domain.auth.us-east-1.amazoncognito.com"
cognito_redirect_uri  = "http://localhost:3000/callback"
```

Then run:

```bash
./target/release/server
```

Environment variables (uppercased key names) override the config file.

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

Config is loaded from `config.toml` first, then overridden by environment variables (uppercased key names).

| Key / Env var | Default | Description |
|---|---|---|
| `kernel_path` / `KERNEL_PATH` | `/var/lib/fc/vmlinux` | Firecracker kernel image |
| `rootfs_path` / `ROOTFS_PATH` | `/var/lib/fc/rootfs.ext4` | Root filesystem image |
| `ssh_key_path` / `SSH_KEY_PATH` | `/var/lib/fc/id_rsa` | SSH private key matching the public key baked into the rootfs |
| `ssh_user` / `SSH_USER` | `root` | SSH login user inside the VM |
| `socket_dir` / `SOCKET_DIR` | `/tmp` | Directory for Firecracker API sockets |
| `port` / `PORT` | `3000` | HTTP listen port |
| `net_helper_path` / `NET_HELPER_PATH` | `/usr/local/bin/net-helper` | Path to the net-helper binary |
| `aws_role_name` / `AWS_ROLE_NAME` | `vm-role` | IAM role name forwarded to the VM via MMDS |
| `cognito_client_id` / `COGNITO_CLIENT_ID` | — | Cognito app client ID |
| `cognito_client_secret` / `COGNITO_CLIENT_SECRET` | — | Cognito app client secret |
| `cognito_region` / `COGNITO_REGION` | — | AWS region of the user pool |
| `cognito_user_pool_id` / `COGNITO_USER_POOL_ID` | — | Cognito user pool ID |
| `cognito_domain` / `COGNITO_DOMAIN` | — | Cognito hosted UI domain |
| `cognito_redirect_uri` / `COGNITO_REDIRECT_URI` | `http://localhost:3000/callback` | OAuth2 redirect URI |

## Networking

The server sets up a `/30` point-to-point network per VM:

| Address | Role |
|---|---|
| `172.16.{n}.1` | Host-side TAP interface |
| `172.16.{n}.2` | VM `eth0` (configured via kernel `ip=` cmdline) |

NAT masquerading is configured automatically on startup via `net-helper setup-nat`.

The VM can reach the internet via the host's default route. The AWS IMDS endpoint (`169.254.169.254`) resolves to the MMDS served by Firecracker.
