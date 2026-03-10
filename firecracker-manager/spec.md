# firecracker-manager

Lifecycle management for Firecracker microVMs: spawn, configure, network, MMDS, and cleanup.

## Responsibilities

- Spawn Firecracker directly or via the jailer (chroot + uid/gid drop)
- Create and tear down TAP network interfaces via `net-helper`
- Configure the VM via `firecracker-client` (boot source, drive, network, MMDS, machine config)
- Copy rootfs into the jailer chroot before boot; save it back on disconnect
- Clean up stale VMs and jailer chroot directories on server startup
- Refresh MMDS IAM credentials for running VMs on a timer

## API

```
create_vm(vm_config: &VmConfig) -> Result<VmGuard>
cleanup_stale_vms(socket_dir: &Path, net_helper_path: &Path, jailer_chroot_base: Option<&Path>)
setup_host_networking(net_helper_path: &Path)
refresh_all_vm_mmds(app_state: &AppState)
build_mmds_with_iam(vm_id: &str, role_name: &str, cred: &ImdsCredential) -> Result<Value>
```

## VmConfig

| Field | Description |
|---|---|
| `id` | UUID string, used as VM identifier and jailer `--id` |
| `socket_dir` | Directory for Firecracker API sockets (non-jailed) |
| `kernel_path` | Host path to the Linux kernel image |
| `rootfs_path` | Host path to the user's rootfs (used directly in non-jailed mode) |
| `net_helper_path` | Path to the `net-helper` binary |
| `vcpu_count` | Number of vCPUs |
| `mem_size_mib` | Guest memory in MiB |
| `boot_args` | Kernel command line |
| `mmds_metadata` | Optional MMDS metadata JSON |
| `mmds_imds_compat` | Enable IMDSv2-compatible MMDS endpoint |
| `jailer` | Optional jailer config; if absent, Firecracker is spawned directly |

## JailerConfig

| Field | Description |
|---|---|
| `jailer_path` | Path to the jailer binary |
| `firecracker_path` | Path to the Firecracker binary (passed to jailer `--exec-file`) |
| `uid` / `gid` | User/group the jailer drops privileges to |
| `chroot_base` | Base directory for jailer chroots (`{chroot_base}/firecracker/{vm_id}/root/`) |

## VmGuard

Holds a running VM. Dropping it sends `SIGTERM`, deletes the TAP interface, and removes the jailer chroot directory (jailed) or the API socket (non-jailed). The rootfs is not deleted on drop — it is the user's persistent file.

```
vm_guard.save_rootfs_to(dest: &Path) -> Result<()>
vm_guard.socket_path() -> &Path
```
