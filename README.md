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

Each user gets their own rootfs with a dedicated SSH keypair baked in.

```bash
ubuntu_version="24.04"

# Unpack upstream squashfs
unsquashfs ubuntu-${ubuntu_version}.squashfs.upstream

# Generate a dedicated SSH keypair for this user's rootfs
ssh-keygen -t ed25519 -f id_rsa -N ""

# Install the public key globally in sshd's config dir (not in ~/.ssh)
# so the ubuntu user cannot delete or replace it
mkdir -p squashfs-root/etc/ssh
cp id_rsa.pub squashfs-root/etc/ssh/authorized_keys
chmod 644 squashfs-root/etc/ssh/authorized_keys

# Point sshd at the global authorized_keys file
sed -i 's|^#*AuthorizedKeysFile.*|AuthorizedKeysFile /etc/ssh/authorized_keys|' \
    squashfs-root/etc/ssh/sshd_config

# Build ext4 image
sudo chown -R root:root squashfs-root
truncate -s 1G ubuntu-${ubuntu_version}.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-${ubuntu_version}.ext4

# Keep the private key alongside the rootfs
mv id_rsa ubuntu-${ubuntu_version}.id_rsa
rm -rf squashfs-root
```

**Why a global `authorized_keys`:** The file lives in `/etc/ssh/` which is root-owned. The `ubuntu` user has no write access there, so they cannot remove or replace the key even with a shell inside the VM.

## Run

```bash
export KERNEL_PATH=/var/lib/fc/vmlinux
export ROOTFS_PATH=/var/lib/fc/ubuntu-24.04.ext4
export SSH_KEY_PATH=/var/lib/fc/ubuntu-24.04.id_rsa
./target/release/server
```

Open http://localhost:3000 — each page load boots a fresh VM and opens an SSH terminal in the browser.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `KERNEL_PATH` | `/var/lib/fc/vmlinux` | Firecracker kernel image |
| `ROOTFS_PATH` | `/var/lib/fc/rootfs.ext4` | Root filesystem image |
| `SSH_KEY_PATH` | `/var/lib/fc/id_rsa` | SSH private key matching the public key baked into the rootfs |
| `SOCKET_DIR` | `/tmp` | Directory for Firecracker API sockets |
| `NET_HELPER_PATH` | `/usr/local/bin/net-helper` | Path to the net-helper binary |
| `AWS_ROLE_NAME` | `vm-role` | IAM role name forwarded to the VM via MMDS |

## Networking

The server sets up a `/30` point-to-point network per VM:

| Address | Role |
|---|---|
| `172.16.{n}.1` | Host-side TAP interface |
| `172.16.{n}.2` | VM `eth0` (configured via kernel `ip=` cmdline) |

NAT masquerading is configured automatically on startup via `net-helper setup-nat`.

The VM can reach the internet via the host's default route. The AWS IMDS endpoint (`169.254.169.254`) resolves to the MMDS served by Firecracker.
