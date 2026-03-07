# vm-terminal

Browser-based terminal that boots a Firecracker microVM per WebSocket connection.

## Prerequisites

- Rust toolchain
- `firecracker` binary on `$PATH`
- `ip` and `iptables` on `$PATH`
- User in the `kvm` group (`sudo usermod -aG kvm $USER`)
- A kernel image and root filesystem for Firecracker

## Build

```bash
cargo build --release
```

## Install net-helper

`net-helper` handles privileged network operations so the server runs without root.

```bash
sudo install -o root -g root -m 0755 target/release/net-helper /usr/local/bin/net-helper
sudo setcap cap_net_admin=eip /usr/local/bin/net-helper
```

## Run

```bash
KERNEL_PATH=/path/to/vmlinux \
ROOTFS_PATH=/path/to/rootfs.ext4 \
./target/release/server
```

Open http://localhost:3000 — each page load boots a fresh VM and connects its console to the browser terminal.

## Guest internet setup

Run these inside the VM to enable outbound internet access:

```bash
ip route add default via 172.16.0.1 dev eth0
echo 'nameserver 1.1.1.1' > /etc/resolv.conf
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `KERNEL_PATH` | `/var/lib/fc/vmlinux` | Firecracker kernel image |
| `ROOTFS_PATH` | `/var/lib/fc/rootfs.ext4` | Root filesystem image |
| `SOCKET_DIR` | `/tmp` | Directory for Firecracker API sockets |
| `NET_HELPER_PATH` | `/usr/local/bin/net-helper` | Path to the net-helper binary |
