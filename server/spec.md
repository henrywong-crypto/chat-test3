# server

Axum web server providing browser-based terminal access to Firecracker microVMs.

## Responsibilities

- Authenticate users via Cognito OAuth2 (or session cookie)
- One VM per user: auto-create on first visit, auto-save rootfs on WebSocket disconnect
- Relay terminal I/O between the browser (xterm.js over WebSocket) and the VM (SSH)
- File upload and download to/from the VM via SFTP
- Persist user rootfs between sessions in `user_rootfs_dir/{user_id}.ext4`
- Inject AWS IAM credentials into the VM via Firecracker MMDS; refresh every 15 minutes

## Routes

```
GET    /                          # redirect to existing VM or create new one → terminal page
GET    /terminal/:id              # terminal page for a running VM
GET    /ws/:id                    # WebSocket SSH relay; drops VM and saves rootfs on close
POST   /sessions/:id/upload       # upload file to VM via SFTP
GET    /sessions/:id/download     # download file from VM via SFTP
POST   /rootfs/delete             # delete the user's saved rootfs (resets disk on next visit)
GET    /login                     # login page
GET    /login/cognito             # initiate Cognito OAuth2 flow
GET    /callback                  # Cognito OAuth2 callback
GET    /logout                    # clear session and redirect to /login
```

## Rootfs lifecycle

1. On visit to `/`: if user has a saved rootfs, use it; otherwise copy from base rootfs
2. VM boots with that rootfs file used directly (no copy in non-jailed mode)
3. On WebSocket disconnect: save rootfs back to `user_rootfs_dir/{user_id}.ext4` under a per-user async lock
4. On server shutdown: save all running VMs' rootfs files

## Configuration

See `config.example.toml`. All fields can also be set via environment variables.

| Key | Default | Description |
|---|---|---|
| `kernel_path` | `/var/lib/fc/vmlinux` | Linux kernel image |
| `rootfs_path` | `/var/lib/fc/rootfs.ext4` | Base rootfs for new users |
| `socket_dir` | `/tmp` | Directory for Firecracker API sockets |
| `net_helper_path` | `/usr/local/bin/net-helper` | Path to net-helper binary |
| `ssh_key_path` | `/var/lib/fc/id_rsa` | SSH private key for VM access |
| `ssh_user` | `root` | SSH user inside the VM |
| `vm_host_key_path` | `/var/lib/fc/vm_host_key.pub` | Expected VM host public key |
| `user_rootfs_dir` | `/home/ubuntu/fc-users` | Per-user rootfs storage directory |
| `upload_dir` | `/home/ubuntu` | Directory inside the VM for uploads/downloads |
| `database_url` | `postgres://localhost/webcode` | PostgreSQL connection URL |
| `port` | `3000` | HTTP listen port |
| `use_jailer` | `false` | Enable Firecracker jailer |
| `jailer_path` | `/usr/local/bin/jailer` | Path to jailer binary |
| `firecracker_path` | `/usr/local/bin/firecracker` | Path to Firecracker binary |
| `jailer_uid` / `jailer_gid` | `0` | UID/GID for jailer to drop privileges to |
| `jailer_chroot_base` | `/srv/jailer` | Base directory for jailer chroots |
